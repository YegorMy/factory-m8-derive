# factory-m8-derive

Derive macros for test data factories with automatic FK resolution. Like FactoryBot for Rust, but with compile-time type safety.

## Why?

If you're using sqlx without an ORM, your test setup probably looks like this:

```rust
let org = sqlx::query_as!(Org, "INSERT INTO org (name) VALUES ($1) RETURNING *", "Acme")
    .fetch_one(&pool).await?;
let user = sqlx::query_as!(User,
    "INSERT INTO users (org_id, email) VALUES ($1, $2) RETURNING *",
    org.id, "test@example.com"
).fetch_one(&pool).await?;
let blog = sqlx::query_as!(Blog,
    "INSERT INTO blog (user_id, title) VALUES ($1, $2) RETURNING *",
    user.id, "My Blog"
).fetch_one(&pool).await?;
let post = sqlx::query_as!(Post,
    "INSERT INTO post (blog_id, title, body) VALUES ($1, $2, $3) RETURNING *",
    blog.id, "Hello", "World"
).fetch_one(&pool).await?;
```

With factory-m8, you just write:

```rust
let post = PostFactory::new()
    .with_title("Hello")
    .with_body("World")
    .create(&pool)
    .await?;
// Org, User, and Blog are created automatically
```

## Installation

```toml
[dev-dependencies]
factory-m8 = { version = "1.0", features = ["derive"] }
async-trait = "0.1"
sqlx = { version = "0.8", features = ["postgres", "runtime-tokio"] }
```

## Full Example

Here's a complete example with a chain of entities: Org → User → Blog → Post.

### 1. Define your ID types and entities

```rust
use factory_m8::{Factory, FactoryCreate, FactoryResult, Sentinel};

// === ID Types ===

#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct OrgId(pub i64);

#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct UserId(pub i64);

#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct BlogId(pub i64);

#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct PostId(pub i64);

// Implement Sentinel so factory-m8 knows when an ID is "unset"
// (0 means "not set, please auto-create")
impl Sentinel for OrgId {
    fn sentinel() -> Self { OrgId(0) }
    fn is_sentinel(&self) -> bool { self.0 == 0 }
}
impl Sentinel for UserId {
    fn sentinel() -> Self { UserId(0) }
    fn is_sentinel(&self) -> bool { self.0 == 0 }
}
impl Sentinel for BlogId {
    fn sentinel() -> Self { BlogId(0) }
    fn is_sentinel(&self) -> bool { self.0 == 0 }
}
impl Sentinel for PostId {
    fn sentinel() -> Self { PostId(0) }
    fn is_sentinel(&self) -> bool { self.0 == 0 }
}

// === Entities ===

pub struct Org {
    pub id: OrgId,
    pub name: String,
}

pub struct User {
    pub id: UserId,
    pub org_id: OrgId,
    pub email: String,
}

pub struct Blog {
    pub id: BlogId,
    pub user_id: UserId,
    pub title: String,
}

pub struct Post {
    pub id: PostId,
    pub blog_id: BlogId,
    pub reviewer_id: Option<UserId>,
    pub title: String,
    pub body: String,
}
```

### 2. Define factories

The `#[fk(...)]` attribute tells the macro how to auto-create dependencies:

```rust
#[derive(Default, Factory)]
#[factory(entity = Org)]
pub struct OrgFactory {
    #[pk]
    pub id: OrgId,
    pub name: String,
}

#[derive(Default, Factory)]
#[factory(entity = User)]
pub struct UserFactory {
    #[pk]
    pub id: UserId,

    #[fk(Org, "id", OrgFactory)]  // if org_id is 0, auto-create an Org
    pub org_id: OrgId,

    pub email: String,
}

#[derive(Default, Factory)]
#[factory(entity = Blog)]
pub struct BlogFactory {
    #[pk]
    pub id: BlogId,

    #[fk(User, "id", UserFactory)]  // if user_id is 0, auto-create a User (which auto-creates Org)
    pub user_id: UserId,

    pub title: String,
}

#[derive(Default, Factory)]
#[factory(entity = Post)]
pub struct PostFactory {
    #[pk]
    pub id: PostId,

    #[fk(Blog, "id", BlogFactory)]  // auto-creates Blog -> User -> Org chain
    pub blog_id: BlogId,

    #[fk(User, "id", UserFactory, no_default)]  // optional FK, won't auto-create
    pub reviewer_id: Option<UserId>,

    pub title: String,
    pub body: String,
}
```

### 3. Implement FactoryCreate

You write the actual INSERT query. The macro generates `build_with_fks()` which resolves all FK dependencies before you insert:

```rust
use async_trait::async_trait;
use sqlx::PgPool;

#[async_trait]
impl FactoryCreate<PgPool> for PostFactory {
    type Entity = Post;

    async fn create(self, pool: &PgPool) -> FactoryResult<Post> {
        // This is where the magic happens:
        // - blog_id is 0? Create a Blog first (which creates User, which creates Org)
        // - reviewer_id is None? Leave it None (no_default flag)
        let e = self.build_with_fks(pool).await?;

        sqlx::query_as!(Post,
            "INSERT INTO post (blog_id, title, body, reviewer_id) VALUES ($1, $2, $3, $4) RETURNING *",
            e.blog_id.0,
            e.title,
            e.body,
            e.reviewer_id.map(|id| id.0)
        ).fetch_one(pool).await.map_err(Into::into)
    }
}
```

### 4. Use in tests

```rust
#[sqlx::test]
async fn test_post_deletion(pool: PgPool) {
    let post = PostFactory::new()
        .with_title("Delete me")
        .create(&pool)
        .await?;

    // org, user, blog all created automatically
    // focus on what you're actually testing

    delete_post(&pool, post.id).await?;
    assert!(get_post(&pool, post.id).await.is_none());
}

#[sqlx::test]
async fn test_posts_same_blog(pool: PgPool) {
    let blog = BlogFactory::new().create(&pool).await?;

    // both posts share the same blog
    let post1 = PostFactory::new().with_blog(&blog).create(&pool).await?;
    let post2 = PostFactory::new().with_blog(&blog).create(&pool).await?;

    assert_eq!(post1.blog_id, post2.blog_id);
}

#[sqlx::test]
async fn test_post_with_reviewer(pool: PgPool) {
    let reviewer = UserFactory::new().create(&pool).await?;

    let post = PostFactory::new()
        .with_reviewer(&reviewer)  // explicitly set optional FK
        .create(&pool)
        .await?;

    assert_eq!(post.reviewer_id, Some(reviewer.id));
}
```

## Generated Methods

The macro generates `with_*` methods for each field:

- `with_<field>(value)` - for regular fields like `with_title("Hello")` or `with_email("test@example.com")`

- `with_<relation>(&entity)` - for FK fields, pass the whole entity: `with_blog(&blog)`. Extracts the ID for you.

- `with_<relation>_id(id)` - if you've only got the ID: `with_blog_id(blog.id)`. Same result, different input.

If you don't call any of these, the factory uses defaults. If an FK field is left at its default (the sentinel value, like `BlogId(0)`), `build_with_fks()` creates that dependency automatically.

| Method | Description |
|--------|-------------|
| `new()` | Create factory with defaults |
| `with_<entity>(&Entity)` | Set FK from entity reference |
| `with_<field>_id(Id)` | Set FK ID directly |
| `with_<field>(value)` | Set field value |
| `build()` | Build entity in-memory |
| `build_with_fks(pool)` | Build entity, auto-creating FK dependencies |

## Attributes

### `#[factory(entity = Type)]`

**Required.** Specifies the entity type this factory creates.

### `#[pk]`

Primary key field. Uses `Default::default()` and no setter is generated.

### `#[fk(Entity, "field", Factory)]`

Foreign key field. Auto-creates the dependency if the value is a sentinel.

```rust
#[fk(User, "id", UserFactory)]
pub user_id: UserId,
```

### `#[fk(Entity, "field", Factory, no_default)]`

Optional FK that won't auto-create. Use for truly optional relationships where you want `None` to stay `None`.

```rust
#[fk(User, "id", UserFactory, no_default)]
pub reviewer_id: Option<UserId>,
```

### `#[required]`

Field that must be set before calling `build()`. Panics if not set.

```rust
#[required]
pub name: Option<String>,
```

## The Sentinel Trait

The `Sentinel` trait detects "unset" values that trigger auto-creation:

```rust
use factory_m8::Sentinel;

#[derive(Clone, Copy, Default)]
pub struct UserId(pub i64);

impl Sentinel for UserId {
    fn sentinel() -> Self { UserId(0) }
    fn is_sentinel(&self) -> bool { self.0 == 0 }
}
```

Common sentinel values:
- Numeric IDs: `0` (database IDs typically start at 1)
- UUIDs: `Uuid::nil()`
- `Option<T>`: `None`

`factory-m8` provides implementations for `i64`, `i32`, `i16`, `u64`, `u32`, `String`, and `Option<T>`.

## Database Backends

Works with any database - implement `FactoryCreate<YourPool>`:

```rust
// PostgreSQL
impl FactoryCreate<sqlx::PgPool> for UserFactory { ... }

// SQLite
impl FactoryCreate<sqlx::SqlitePool> for UserFactory { ... }

// MongoDB
impl FactoryCreate<mongodb::Database> for UserFactory { ... }
```

## Mixed Backends

For projects using multiple databases, use `no_default` on cross-backend FKs:

```rust
#[derive(Factory)]
#[factory(entity = Patient)]
pub struct PatientFactory {
    // Postgres - auto-creates
    #[fk(Practice, "id", PracticeFactory)]
    pub practice_id: PracticeId,

    // MongoDB - set manually, no auto-create
    #[fk(AuditLog, "id", AuditLogFactory, no_default)]
    pub audit_log_id: Option<AuditLogId>,
}
```

## License

MIT License - see [LICENSE](LICENSE) for details.
