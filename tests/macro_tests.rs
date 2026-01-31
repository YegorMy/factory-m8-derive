//! Tests for the Factory derive macro.
//!
//! These tests demonstrate what the macro generates and how to use it.

use async_trait::async_trait;
use factory_m8::{FactoryCreate, Sentinel};
use factory_derive::Factory;
use std::error::Error;

// =============================================================================
// MOCK DATABASE POOL (for unit tests without real DB)
// =============================================================================

/// A mock pool type for testing. In real code you'd use sqlx::PgPool, etc.
pub struct MockPool;

// =============================================================================
// SIMPLE ID TYPES (demonstrating Sentinel trait usage without typed-ids)
// =============================================================================

/// Simple newtype wrapper for i64 IDs that implements Sentinel.
/// This demonstrates how users can define their own ID types.
macro_rules! define_simple_id {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
        pub struct $name(pub i64);

        impl Sentinel for $name {
            fn sentinel() -> Self {
                $name(0)
            }

            fn is_sentinel(&self) -> bool {
                self.0 == 0
            }
        }
    };
}

define_simple_id!(PracticeId);
define_simple_id!(TenantId);
define_simple_id!(PatientId);

// =============================================================================
// MOCK ENTITIES
// =============================================================================

#[derive(Debug, Clone)]
pub struct Practice {
    pub id: PracticeId,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct Tenant {
    pub id: TenantId,
    pub name: String,
}

// =============================================================================
// MOCK FACTORIES FOR FK RESOLUTION
// =============================================================================

/// Mock factory for Practice - used when PatientFactory auto-creates practice
#[derive(Debug, Default)]
pub struct PracticeFactory {
    pub id: PracticeId,
    pub name: Option<String>,
}

#[async_trait]
impl FactoryCreate<MockPool> for PracticeFactory {
    type Entity = Practice;

    async fn create(self, _pool: &MockPool) -> Result<Practice, Box<dyn Error + Send + Sync>> {
        // In real tests this would INSERT into DB
        // For unit tests, just return a mock entity
        Ok(Practice {
            id: PracticeId(999), // Mock auto-generated ID
            name: self
                .name
                .unwrap_or_else(|| "Auto-created Practice".to_string()),
        })
    }
}

impl PracticeFactory {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Mock factory for Tenant
#[derive(Debug, Default)]
pub struct TenantFactory {
    pub id: TenantId,
    pub name: Option<String>,
}

#[async_trait]
impl FactoryCreate<MockPool> for TenantFactory {
    type Entity = Tenant;

    async fn create(self, _pool: &MockPool) -> Result<Tenant, Box<dyn Error + Send + Sync>> {
        Ok(Tenant {
            id: TenantId(888),
            name: self
                .name
                .unwrap_or_else(|| "Auto-created Tenant".to_string()),
        })
    }
}

impl TenantFactory {
    pub fn new() -> Self {
        Self::default()
    }
}

// =============================================================================
// TEST 1: Basic factory with FK fields
// =============================================================================

/// Entity with practice_id (FK) and optional fields
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Patient {
    pub id: PatientId,
    pub practice_id: PracticeId,
    pub tenant_id: Option<TenantId>,
    pub first_name: Option<String>,
}

/// Factory with FK - all FK fields are Option<Id>
#[derive(Debug, Default, Factory)]
#[factory(entity = Patient)]
pub struct PatientFactory {
    #[pk]
    pub id: PatientId,

    #[fk(Practice, "id", PracticeFactory)]
    pub practice_id: PracticeId,

    #[fk(Tenant, "id", TenantFactory)] // Entity has Option<TenantId>
    pub tenant_id: Option<TenantId>,

    pub first_name: Option<String>,
}

#[test]
fn test_new_returns_default_factory() {
    let factory = PatientFactory::new();

    // Default PracticeId is sentinel (0)
    assert!(factory.practice_id.is_sentinel());
    assert!(factory.tenant_id.is_none());
    assert!(factory.first_name.is_none());
}

#[test]
fn test_with_practice_sets_id_from_entity() {
    let practice = Practice {
        id: PracticeId(123),
        name: "Test Practice".to_string(),
    };

    let factory = PatientFactory::new().with_practice(&practice);

    assert_eq!(factory.practice_id, PracticeId(123));
}

#[test]
fn test_with_practice_id_sets_id_directly() {
    let factory = PatientFactory::new().with_practice_id(PracticeId(456));

    assert_eq!(factory.practice_id, PracticeId(456));
}

#[test]
fn test_with_optional_fields() {
    let practice = Practice {
        id: PracticeId(1),
        name: "Practice".to_string(),
    };
    let tenant = Tenant {
        id: TenantId(2),
        name: "Tenant".to_string(),
    };

    let factory = PatientFactory::new()
        .with_practice(&practice)
        .with_tenant(&tenant)
        .with_first_name("Alice");

    assert_eq!(factory.practice_id, PracticeId(1));
    assert_eq!(factory.tenant_id, Some(TenantId(2)));
    assert_eq!(factory.first_name, Some("Alice".to_string()));
}

#[test]
fn test_build_creates_entity_when_fks_set() {
    let practice = Practice {
        id: PracticeId(42),
        name: "Build Test".to_string(),
    };

    let patient = PatientFactory::new()
        .with_practice(&practice)
        .with_first_name("Bob")
        .build();

    assert_eq!(patient.id, PatientId(0)); // pk_i64 defaults
    assert_eq!(patient.practice_id, PracticeId(42));
    assert_eq!(patient.first_name, Some("Bob".to_string()));
    assert_eq!(patient.tenant_id, None); // Optional FK stays None
}

// =============================================================================
// TEST 2: Factory with #[required] non-FK field
// =============================================================================

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PatientWithRequiredName {
    pub id: PatientId,
    pub practice_id: PracticeId,
    pub name: String, // Required!
    pub nickname: Option<String>,
}

#[derive(Debug, Default, Factory)]
#[factory(entity = PatientWithRequiredName)]
pub struct PatientWithRequiredNameFactory {
    #[pk]
    pub id: PatientId,

    #[fk(Practice, "id", PracticeFactory)]
    pub practice_id: PracticeId,

    #[required]
    pub name: Option<String>,

    pub nickname: Option<String>,
}

#[test]
fn test_required_field_works_when_set() {
    let practice = Practice {
        id: PracticeId(1),
        name: "Test".to_string(),
    };

    let entity = PatientWithRequiredNameFactory::new()
        .with_practice(&practice)
        .with_name("Required Name")
        .build();

    assert_eq!(entity.name, "Required Name");
    assert_eq!(entity.nickname, None);
}

#[test]
#[should_panic(expected = "name is required")]
fn test_required_field_panics_when_missing() {
    let practice = Practice {
        id: PracticeId(1),
        name: "Test".to_string(),
    };

    PatientWithRequiredNameFactory::new()
        .with_practice(&practice)
        // name not set!
        .build();
}

// =============================================================================
// TEST 3: Factory with ALL OPTIONAL fields (no FK, no required)
// =============================================================================

#[derive(Debug, Clone, PartialEq, Default)]
pub struct AllOptionalEntity {
    pub id: PatientId,
    pub name: Option<String>,
    pub age: Option<i32>,
}

#[derive(Debug, Default, Factory)]
#[factory(entity = AllOptionalEntity)]
pub struct AllOptionalFactory {
    #[pk]
    pub id: PatientId,
    pub name: Option<String>,
    pub age: Option<i32>,
}

#[test]
fn test_all_optional_factory() {
    let factory = AllOptionalFactory::new().with_name("Test").with_age(25);

    assert_eq!(factory.name, Some("Test".to_string()));
    assert_eq!(factory.age, Some(25));
}

#[test]
fn test_all_optional_build_with_none() {
    let entity = AllOptionalFactory::new().build();

    assert_eq!(entity.id, PatientId(0));
    assert_eq!(entity.name, None);
    assert_eq!(entity.age, None);
}

// =============================================================================
// TEST 4: Factory with NON-OPTION FK field (FK field is IdType, not Option<IdType>)
// =============================================================================

/// Entity where the FK field is NOT optional - practice_id is PracticeId, not Option<PracticeId>
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EntityWithRequiredFk {
    pub id: PatientId,
    pub practice_id: PracticeId, // Required FK - NOT Option
    pub name: Option<String>,
}

/// Factory with non-Option FK field - the FK field type matches the entity exactly
#[derive(Debug, Default, Factory)]
#[factory(entity = EntityWithRequiredFk)]
pub struct EntityWithRequiredFkFactory {
    #[pk]
    pub id: PatientId,

    #[fk(Practice, "id", PracticeFactory)]
    pub practice_id: PracticeId, // Non-Option FK field - same as entity

    pub name: Option<String>,
}

#[test]
fn test_non_option_fk_with_entity_setter() {
    let practice = Practice {
        id: PracticeId(777),
        name: "Non-Option FK Test".to_string(),
    };

    let factory = EntityWithRequiredFkFactory::new().with_practice(&practice);

    // The FK is set directly (not wrapped in Some)
    assert_eq!(factory.practice_id, PracticeId(777));
}

#[test]
fn test_non_option_fk_with_id_setter() {
    let factory = EntityWithRequiredFkFactory::new().with_practice_id(PracticeId(888));

    assert_eq!(factory.practice_id, PracticeId(888));
}

#[test]
fn test_non_option_fk_build_uses_value_directly() {
    let practice = Practice {
        id: PracticeId(999),
        name: "Build Test".to_string(),
    };

    let entity = EntityWithRequiredFkFactory::new()
        .with_practice(&practice)
        .with_name("Test Entity")
        .build();

    assert_eq!(entity.practice_id, PracticeId(999));
    assert_eq!(entity.name, Some("Test Entity".to_string()));
}

#[test]
fn test_non_option_fk_build_with_default() {
    // Non-Option FK uses Default value if not explicitly set
    let entity = EntityWithRequiredFkFactory::new().build();

    // Default for PracticeId is PracticeId(0)
    assert_eq!(entity.practice_id, PracticeId(0));
}

// =============================================================================
// WHAT THE MACRO GENERATES (for reference)
// =============================================================================
//
// For PatientFactory, the macro generates:
//
// impl PatientFactory {
//     pub fn new() -> Self {
//         Self::default()
//     }
//
//     // FK: entity reference setter
//     pub fn with_practice(mut self, entity: &Practice) -> Self {
//         self.practice_id = Some(entity.id);
//         self
//     }
//
//     // FK: direct ID setter
//     pub fn with_practice_id(mut self, id: PracticeId) -> Self {
//         self.practice_id = Some(id);
//         self
//     }
//
//     // Optional field setter
//     pub fn with_first_name(mut self, value: impl Into<String>) -> Self {
//         self.first_name = Some(value.into());
//         self
//     }
//
//     // Panics if required fields missing
//     pub fn build(&self) -> Patient {
//         Patient {
//             id: Default::default(),
//             practice_id: self.practice_id.expect("practice_id is required..."),
//             tenant_id: self.tenant_id.clone(),
//             first_name: self.first_name.clone(),
//         }
//     }
//
//     // Auto-creates FK dependencies if missing (generic over Pool)
//     pub async fn build_with_fks<Pool>(&self, pool: &Pool) -> Result<Patient, Box<dyn Error>>
//     where
//         Pool: Sync,
//         PracticeFactory: FactoryCreate<Pool>,
//         TenantFactory: FactoryCreate<Pool>,
//     {
//         let resolved_practice_id = if self.practice_id.is_sentinel() {
//             let entity = PracticeFactory::new().create(pool).await?;
//             entity.id
//         } else {
//             self.practice_id
//         };
//         // ... similar for other FKs
//         Ok(Patient { ... })
//     }
// }
