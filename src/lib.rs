//! Factory Derive Macros
//!
//! Generates factory boilerplate for test entities with automatic FK resolution.
//!
//! ## Example Usage
//!
//! ```ignore
//! use factory_m8::FactoryCreate;
//! use factory_derive::Factory;
//!
//! #[derive(Debug, Default, Factory)]
//! #[factory(entity = Patient)]
//! pub struct PatientFactory {
//!     #[pk]
//!     pub id: PatientId,
//!
//!     // Option<T> FK - auto-creates if None/sentinel, result is Some(id)
//!     // Factory field type should match entity field type
//!     #[fk(Practice, "id", PracticeFactory)]
//!     pub practice_id: Option<PracticeId>,
//!
//!     // Non-Option FK - auto-creates if is_sentinel() returns true
//!     // Default impl typically sets to sentinel value (e.g., Id(0))
//!     #[fk(Tenant, "id", TenantFactory)]
//!     pub tenant_id: TenantId,
//!
//!     // Option<T> FK with no_default - won't auto-create, None stays None
//!     // Use for truly optional FKs where entity field is also Option
//!     #[fk(Provider, "id", ProviderFactory, no_default)]
//!     pub provider_id: Option<ProviderId>,
//!
//!     // Non-Option field - used directly (provide in Default impl)
//!     pub name: String,
//!
//!     // Option field - cloned as-is (truly optional)
//!     pub nickname: Option<String>,
//! }
//!
//! // User implements just the INSERT
//! impl FactoryCreate for PatientFactory {
//!     type Entity = Patient;
//!
//!     async fn create(self, pool: &PgPool) -> Result<Patient, Box<dyn Error + Send + Sync>> {
//!         let entity = self.build_with_fks(pool).await?;
//!         sqlx::query_as!(Patient, "INSERT INTO patient ...")
//!             .fetch_one(pool).await
//!     }
//! }
//! ```
//!
//! ## Attributes
//!
//! - `#[factory(entity = EntityType)]` - Specifies the entity type this factory creates
//! - `#[pk]` - Primary key field, uses Default::default()
//! - `#[fk(Entity, "field", Factory)]` - FK field, optionality based on field type:
//!   - `Option<T>`: auto-creates if None/unset, returns `Some(id)`
//!   - `T` (non-Option): auto-creates if `is_unset()`, returns `id`
//! - `#[fk(Entity, "field", Factory, no_default)]` - Don't auto-create, None stays None
//!
//! ## FK Field Types
//!
//! FK field type determines behavior in `build_with_fks()`:
//!
//! - `Option<IdType>`: Auto-creates if None or sentinel, returns `Some(created_id)`.
//!   Use `no_default` flag to disable auto-creation (None stays None).
//!
//! - `IdType` (non-Option): Auto-creates if `is_sentinel()` returns true.
//!   Default impl should set to sentinel value (e.g., `Id(0)`).
//!
//! **Important**: Factory field type should match entity field type.
//!
//! ## Generated Methods
//!
//! - `new()` - Creates factory with default values
//! - `with_<entity>(&Entity)` - Sets FK from entity reference
//! - `with_<field>_id(Id)` - Sets FK ID directly
//! - `with_<field>(value)` - Sets field value (for Option and non-Option fields)
//! - `build()` - Creates entity in-memory (clones Option FK fields as-is)
//! - `build_with_fks(pool)` - Creates entity, auto-creating FK dependencies if needed

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{
    parse_macro_input, Data, DeriveInput, Expr, Field, Fields, Ident, LitStr, Meta, Token, Type,
};

// =============================================================================
// MAIN DERIVE MACRO
// =============================================================================

#[proc_macro_derive(Factory, attributes(factory, fk, pk, required))]
pub fn derive_factory(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    let factory_name = &input.ident;

    // Parse #[factory(entity = EntityType)]
    let entity_type =
        parse_factory_attr(&input).expect("Missing #[factory(entity = EntityType)] attribute");

    // Get struct fields
    let fields = match &input.data {
        Data::Struct(s) => match &s.fields {
            Fields::Named(f) => &f.named,
            _ => panic!("Factory only supports named fields"),
        },
        _ => panic!("Factory only works on structs"),
    };

    let fields_vec: Vec<&Field> = fields.iter().collect();

    // Categorize fields
    let fk_fields: Vec<&Field> = fields_vec
        .iter()
        .filter(|f| parse_fk_attr(f).is_some())
        .copied()
        .collect();

    // Non-PK, non-FK fields that are Option<T>
    let option_non_fk_fields: Vec<&Field> = fields_vec
        .iter()
        .filter(|f| !has_attr(f, "pk"))
        .filter(|f| parse_fk_attr(f).is_none())
        .filter(|f| is_option_type(&f.ty))
        .copied()
        .collect();

    // Non-PK, non-FK fields that are NOT Option<T> (regular fields)
    let regular_non_fk_fields: Vec<&Field> = fields_vec
        .iter()
        .filter(|f| !has_attr(f, "pk"))
        .filter(|f| parse_fk_attr(f).is_none())
        .filter(|f| !is_option_type(&f.ty))
        .copied()
        .collect();

    // Generate with_* methods for FK fields (two versions: entity ref and direct ID)
    let fk_with_methods: Vec<TokenStream2> = fk_fields
        .iter()
        .flat_map(|f| generate_fk_with_methods(f))
        .collect();

    // Generate with_* methods for Option non-FK fields
    let option_with_methods: Vec<TokenStream2> = option_non_fk_fields
        .iter()
        .map(|f| generate_option_with_method(f))
        .collect();

    // Generate with_* methods for regular (non-Option) non-FK fields
    let regular_with_methods: Vec<TokenStream2> = regular_non_fk_fields
        .iter()
        .map(|f| generate_regular_with_method(f))
        .collect();

    // Generate build() field assignments
    let build_assignments: Vec<TokenStream2> = fields_vec
        .iter()
        .map(|f| generate_build_assignment(f))
        .collect();

    // Generate build_with_fks() FK resolution
    let fk_resolutions: Vec<TokenStream2> = fk_fields
        .iter()
        .map(|f| generate_fk_resolution(f))
        .collect();

    // Generate build_with_fks() field assignments
    let build_with_fks_assignments: Vec<TokenStream2> = fields_vec
        .iter()
        .map(|f| generate_build_with_fks_assignment(f))
        .collect();

    // Collect FK factory types that need FactoryCreate<Pool> bounds
    // (only those without no_default, as those are the ones that auto-create)
    // We constrain both the factory trait AND the associated Entity type
    let fk_factory_bounds: Vec<TokenStream2> = fk_fields
        .iter()
        .filter_map(|f| {
            let fk_info = parse_fk_attr(f)?;
            if fk_info.no_default {
                None // no_default FKs don't auto-create, no bound needed
            } else {
                let factory_type = fk_info.factory_type;
                let entity_type = fk_info.entity_type;
                // Constrain that the factory's Entity type matches the expected entity
                Some(quote! { #factory_type: factory_m8::FactoryCreate<Pool, Entity = #entity_type> })
            }
        })
        .collect();

    // Generate the impl block
    let expanded = if fk_factory_bounds.is_empty() {
        // No FK auto-creation, simpler signature without bounds
        quote! {
            impl #factory_name {
                /// Create a new factory with default values.
                pub fn new() -> Self {
                    Self::default()
                }

                #(#fk_with_methods)*

                #(#option_with_methods)*

                #(#regular_with_methods)*

                /// Build an in-memory entity without DB insert.
                /// Panics if required FK fields are None.
                pub fn build(&self) -> #entity_type {
                    #entity_type {
                        #(#build_assignments),*
                    }
                }

                /// Build entity with automatic FK resolution.
                /// Generic over the database pool type.
                pub async fn build_with_fks<Pool>(
                    &self,
                    _pool: &Pool,
                ) -> Result<#entity_type, Box<dyn std::error::Error + Send + Sync>>
                where
                    Pool: Sync,
                {
                    // No FK resolutions needed
                    #(#fk_resolutions)*

                    Ok(#entity_type {
                        #(#build_with_fks_assignments),*
                    })
                }
            }
        }
    } else {
        // Has FK auto-creation, need bounds for FK factories
        quote! {
            impl #factory_name {
                /// Create a new factory with default values.
                pub fn new() -> Self {
                    Self::default()
                }

                #(#fk_with_methods)*

                #(#option_with_methods)*

                #(#regular_with_methods)*

                /// Build an in-memory entity without DB insert.
                /// Panics if required FK fields are None.
                pub fn build(&self) -> #entity_type {
                    #entity_type {
                        #(#build_assignments),*
                    }
                }

                /// Build entity with automatic FK resolution.
                /// If FK fields are sentinel values, creates dependencies via their factories.
                ///
                /// Generic over the database pool type - works with any backend
                /// (sqlx::PgPool, sqlx::SqlitePool, mongodb::Database, etc.)
                pub async fn build_with_fks<Pool>(
                    &self,
                    pool: &Pool,
                ) -> Result<#entity_type, Box<dyn std::error::Error + Send + Sync>>
                where
                    Pool: Sync,
                    #(#fk_factory_bounds,)*
                {
                    // Resolve all FK dependencies
                    #(#fk_resolutions)*

                    Ok(#entity_type {
                        #(#build_with_fks_assignments),*
                    })
                }
            }
        }
    };

    TokenStream::from(expanded)
}

// =============================================================================
// ATTRIBUTE PARSING
// =============================================================================

/// Parses #[factory(entity = EntityType)]
fn parse_factory_attr(input: &DeriveInput) -> Option<Ident> {
    for attr in &input.attrs {
        if attr.path().is_ident("factory") {
            let nested = attr
                .parse_args_with(syn::punctuated::Punctuated::<Meta, Token![,]>::parse_terminated)
                .ok()?;

            for meta in nested {
                if let Meta::NameValue(nv) = meta {
                    if nv.path.is_ident("entity") {
                        if let Expr::Path(expr_path) = &nv.value {
                            return expr_path.path.get_ident().cloned();
                        }
                    }
                }
            }
        }
    }
    None
}

/// FK attribute info
struct FkAttrInfo {
    entity_type: Ident,
    entity_field: Ident,
    factory_type: Ident,
    /// When true, don't auto-create FK dependency (None stays None for Option fields)
    no_default: bool,
}

/// Parses #[fk(EntityType, "field", FactoryType)] or #[fk(EntityType, "field", FactoryType, no_default)]
///
/// The optionality of the FK is determined by the field type:
/// - `Option<T>`: Optional FK, auto-creates if None/sentinel (unless `no_default` is set)
/// - `T` (non-Option): Required FK, auto-creates if is_sentinel()
///
/// The `no_default` flag prevents auto-creation: None/sentinel stays None for Option fields.
fn parse_fk_attr(field: &Field) -> Option<FkAttrInfo> {
    for attr in &field.attrs {
        if attr.path().is_ident("fk") {
            let result = attr.parse_args_with(|input: syn::parse::ParseStream| {
                let entity_type: Ident = input.parse()?;
                input.parse::<Token![,]>()?;
                let field_name_lit: LitStr = input.parse()?;
                let entity_field = Ident::new(&field_name_lit.value(), field_name_lit.span());
                input.parse::<Token![,]>()?;
                let factory_type: Ident = input.parse()?;

                // Check for no_default flag
                let no_default = if input.peek(Token![,]) {
                    input.parse::<Token![,]>()?;
                    let flag: Ident = input.parse()?;
                    flag == "no_default"
                } else {
                    false
                };

                Ok(FkAttrInfo {
                    entity_type,
                    entity_field,
                    factory_type,
                    no_default,
                })
            });
            return result.ok();
        }
    }
    None
}

/// Checks if field has a specific attribute
fn has_attr(field: &Field, name: &str) -> bool {
    field.attrs.iter().any(|a| a.path().is_ident(name))
}

// =============================================================================
// CODE GENERATION: with_* methods for FK fields
// =============================================================================

/// Generates two with methods for FK fields:
/// - with_<entity>(&Entity) - sets ID from entity reference
/// - with_<field>_id(Id) - sets ID directly
///
/// Supports both Option<IdType> and IdType FK fields.
fn generate_fk_with_methods(field: &Field) -> Vec<TokenStream2> {
    let field_name = field.ident.as_ref().unwrap();
    let fk_info = parse_fk_attr(field).unwrap();

    let entity_type = &fk_info.entity_type;
    let entity_field = &fk_info.entity_field;

    // Method name: practice_id -> with_practice
    let entity_method_name = fk_method_name(field_name);
    // Method name: practice_id -> with_practice_id
    let id_method_name = format_ident!("with_{}", field_name);

    // Check if FK field is Option<IdType> or just IdType
    if let Some(id_type) = extract_option_inner_type(&field.ty) {
        // Option<IdType> - wrap in Some
        vec![
            quote! {
                /// Set FK from entity reference.
                pub fn #entity_method_name(mut self, entity: &#entity_type) -> Self {
                    self.#field_name = Some(entity.#entity_field);
                    self
                }
            },
            quote! {
                /// Set FK ID directly.
                pub fn #id_method_name(mut self, id: #id_type) -> Self {
                    self.#field_name = Some(id);
                    self
                }
            },
        ]
    } else {
        // Non-Option IdType - use directly
        let field_type = &field.ty;
        vec![
            quote! {
                /// Set FK from entity reference.
                pub fn #entity_method_name(mut self, entity: &#entity_type) -> Self {
                    self.#field_name = entity.#entity_field;
                    self
                }
            },
            quote! {
                /// Set FK ID directly.
                pub fn #id_method_name(mut self, id: #field_type) -> Self {
                    self.#field_name = id;
                    self
                }
            },
        ]
    }
}

/// Converts FK field name to entity method name:
/// - practice_id -> with_practice
/// - procedure_id_origin -> with_procedure_origin (replaces _id_ with _)
/// - tenant_id -> with_tenant
fn fk_method_name(field_name: &Ident) -> Ident {
    let name = field_name.to_string();
    // First try stripping _id suffix (common case like practice_id)
    if let Some(stripped) = name.strip_suffix("_id") {
        return format_ident!("with_{}", stripped);
    }
    // Otherwise replace _id_ with _ (for fields like procedure_id_origin)
    let stripped = name.replace("_id_", "_");
    format_ident!("with_{}", stripped)
}

// =============================================================================
// CODE GENERATION: with_* methods for Option non-FK fields
// =============================================================================

fn generate_option_with_method(field: &Field) -> TokenStream2 {
    let field_name = field.ident.as_ref().unwrap();
    let field_type = &field.ty;
    let method_name = format_ident!("with_{}", field_name);

    let inner_type = extract_option_inner_type(field_type).expect("Option field must be Option<T>");

    if is_string_type(inner_type) {
        quote! {
            /// Set optional field value.
            pub fn #method_name(mut self, value: impl Into<String>) -> Self {
                self.#field_name = Some(value.into());
                self
            }
        }
    } else {
        quote! {
            /// Set optional field value.
            pub fn #method_name(mut self, value: #inner_type) -> Self {
                self.#field_name = Some(value);
                self
            }
        }
    }
}

// =============================================================================
// CODE GENERATION: with_* methods for regular (non-Option) non-FK fields
// =============================================================================

fn generate_regular_with_method(field: &Field) -> TokenStream2 {
    let field_name = field.ident.as_ref().unwrap();
    let field_type = &field.ty;
    let method_name = format_ident!("with_{}", field_name);

    if is_string_type(field_type) {
        quote! {
            /// Set field value.
            pub fn #method_name(mut self, value: impl Into<String>) -> Self {
                self.#field_name = value.into();
                self
            }
        }
    } else {
        quote! {
            /// Set field value.
            pub fn #method_name(mut self, value: #field_type) -> Self {
                self.#field_name = value;
                self
            }
        }
    }
}

// =============================================================================
// CODE GENERATION: build() assignments
// =============================================================================

fn generate_build_assignment(field: &Field) -> TokenStream2 {
    let field_name = field.ident.as_ref().unwrap();
    let field_name_str = field_name.to_string();

    // pk: use Default
    if has_attr(field, "pk") {
        return quote! {
            #field_name: Default::default()
        };
    }

    // FK field: behavior based on field type
    if let Some(_fk_info) = parse_fk_attr(field) {
        let is_option_field = is_option_type(&field.ty);

        if is_option_field {
            // Option<T> FK field: clone as-is for build() (entity field is Option<T>)
            return quote! {
                #field_name: self.#field_name.clone()
            };
        } else {
            // Required FK with non-Option factory field: use directly
            if needs_clone(&field.ty) {
                return quote! {
                    #field_name: self.#field_name.clone()
                };
            } else {
                return quote! {
                    #field_name: self.#field_name
                };
            }
        }
    }

    // #[required] Option field: unwrap with error message (entity field is non-Option)
    if has_attr(field, "required") && is_option_type(&field.ty) {
        let error_msg = format!("{field_name_str} is required - use with_{field_name_str}()");
        return quote! {
            #field_name: self.#field_name.clone().expect(#error_msg)
        };
    }

    // Option field: clone as-is
    if is_option_type(&field.ty) {
        return quote! {
            #field_name: self.#field_name.clone()
        };
    }

    // Regular non-Option field: clone or copy
    if needs_clone(&field.ty) {
        quote! {
            #field_name: self.#field_name.clone()
        }
    } else {
        quote! {
            #field_name: self.#field_name
        }
    }
}

// =============================================================================
// CODE GENERATION: build_with_fks() FK resolution
// =============================================================================

fn generate_fk_resolution(field: &Field) -> TokenStream2 {
    let field_name = field.ident.as_ref().unwrap();
    let fk_info = parse_fk_attr(field).unwrap();
    let entity_type = &fk_info.entity_type;
    let entity_field = &fk_info.entity_field;
    let factory_type = &fk_info.factory_type;
    let is_option_field = is_option_type(&field.ty);

    // Variable name for resolved ID
    let resolved_var = format_ident!("resolved_{}", field_name);

    if is_option_field {
        if fk_info.no_default {
            // Option<T> with no_default: don't auto-create, None/sentinel stays None
            // Returns Option<T> - for truly optional entity fields
            quote! {
                let #resolved_var = {
                    use factory_m8::Sentinel;
                    match self.#field_name {
                        Some(id) if !id.is_sentinel() => Some(id),
                        _ => None,  // None or Some(sentinel) stays None
                    }
                };
            }
        } else {
            // Option<T> without no_default: auto-create if None/sentinel
            // Returns Option<T> (Some(id)) - for Option entity fields
            quote! {
                let #resolved_var = {
                    use factory_m8::Sentinel;
                    Some(match self.#field_name {
                        Some(id) if !id.is_sentinel() => id,
                        _ => {
                            // Auto-create dependency via factory
                            use factory_m8::FactoryCreate;
                            let entity: #entity_type = #factory_type::new().create(pool).await?;
                            entity.#entity_field
                        }
                    })
                };
            }
        }
    } else {
        // Non-Option field: auto-create if sentinel (no_default doesn't apply)
        // Returns T
        quote! {
            let #resolved_var = {
                use factory_m8::Sentinel;
                if self.#field_name.is_sentinel() {
                    // Auto-create dependency via factory
                    use factory_m8::FactoryCreate;
                    let entity: #entity_type = #factory_type::new().create(pool).await?;
                    entity.#entity_field
                } else {
                    self.#field_name
                }
            };
        }
    }
}

fn generate_build_with_fks_assignment(field: &Field) -> TokenStream2 {
    let field_name = field.ident.as_ref().unwrap();

    // pk: use Default
    if has_attr(field, "pk") {
        return quote! {
            #field_name: Default::default()
        };
    }

    // FK field: use resolved variable
    // The resolved variable type matches the field type (Option<T> or T)
    if parse_fk_attr(field).is_some() {
        let resolved_var = format_ident!("resolved_{}", field_name);
        return quote! {
            #field_name: #resolved_var
        };
    }

    // #[required] Option field: unwrap (entity field is non-Option)
    let field_name_str = field_name.to_string();
    if has_attr(field, "required") && is_option_type(&field.ty) {
        let error_msg = format!("{field_name_str} is required - use with_{field_name_str}()");
        return quote! {
            #field_name: self.#field_name.clone().expect(#error_msg)
        };
    }

    // Option field: clone as-is
    if is_option_type(&field.ty) {
        return quote! {
            #field_name: self.#field_name.clone()
        };
    }

    // Regular non-Option field: clone or copy
    if needs_clone(&field.ty) {
        quote! {
            #field_name: self.#field_name.clone()
        }
    } else {
        quote! {
            #field_name: self.#field_name
        }
    }
}

// =============================================================================
// TYPE HELPERS
// =============================================================================

fn is_option_type(ty: &Type) -> bool {
    extract_option_inner_type(ty).is_some()
}

fn extract_option_inner_type(ty: &Type) -> Option<&Type> {
    if let Type::Path(type_path) = ty {
        let segment = type_path.path.segments.last()?;
        if segment.ident == "Option" {
            if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                if let Some(syn::GenericArgument::Type(inner)) = args.args.first() {
                    return Some(inner);
                }
            }
        }
    }
    None
}

fn is_string_type(ty: &Type) -> bool {
    if let Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.last() {
            return segment.ident == "String";
        }
    }
    false
}

fn needs_clone(ty: &Type) -> bool {
    if let Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.last() {
            let name = segment.ident.to_string();
            return !matches!(
                name.as_str(),
                "bool"
                    | "i8"
                    | "i16"
                    | "i32"
                    | "i64"
                    | "i128"
                    | "u8"
                    | "u16"
                    | "u32"
                    | "u64"
                    | "u128"
                    | "f32"
                    | "f64"
                    | "char"
            );
        }
    }
    true
}
