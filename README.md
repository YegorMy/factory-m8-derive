# factory-m8-derive

Derive macros for test data factories with automatic FK resolution.

## Installation

```toml
[dev-dependencies]
factory-m8-derive = "0.1"
factory-m8 = "0.1"
async-trait = "0.1"
sqlx = { version = "0.8", features = ["postgres", "runtime-tokio"] }
```

## Quick Start

```rust
use factory_m8::{FactoryCreate, Sentinel};
use factory_m8_derive::Factory;
use async_trait::async_trait;
use sqlx::PgPool;

// 1. Define your entity
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Patient {
    pub id: i64,
    pub practice_id: i64,
    pub name: String,
}

// 2. Define the factory
#[derive(Debug, Default, Factory)]
#[factory(entity = Patient)]
pub struct PatientFactory {
    #[pk]
    pub id: i64,

    #[fk(Practice, "id", PracticeFactory)]
    pub practice_id: i64,  // Auto-creates Practice if sentinel (0)

    #[required]
    pub name: Option<String>,
}

// 3. Implement FactoryCreate
#[async_trait]
impl FactoryCreate<PgPool> for PatientFactory {
    type Entity = Patient;

    async fn create(self, pool: &PgPool) -> Result<Patient, Box<dyn std::error::Error + Send + Sync>> {
        let entity = self.build_with_fks(pool).await?;

        sqlx::query_as::<_, Patient>(
            "INSERT INTO patient (practice_id, name) VALUES ($1, $2) RETURNING *"
        )
        .bind(entity.practice_id)
        .bind(&entity.name)
        .fetch_one(pool)
        .await
        .map_err(Into::into)
    }
}

// 4. Use in tests
#[sqlx::test]
async fn test_create_patient(pool: PgPool) {
    // Practice is auto-created!
    let patient = PatientFactory::new()
        .with_name("John Doe")
        .create(&pool)
        .await
        .unwrap();

    assert_eq!(patient.name, "John Doe");
}
```

## Attributes

### `#[factory(entity = Type)]`

**Required.** Specifies the entity type this factory creates.

### `#[pk]`

Primary key field. Uses `Default::default()` and no setter is generated.

### `#[fk(Entity, "field", Factory)]`

Foreign key field. Auto-creates the dependency if the value is a sentinel.

```rust
#[fk(Practice, "id", PracticeFactory)]
pub practice_id: PracticeId,
```

### `#[fk(Entity, "field", Factory, no_default)]`

Optional FK that won't auto-create. Use for truly optional relationships.

```rust
#[fk(Referrer, "id", ReferrerFactory, no_default)]
pub referrer_id: Option<ReferrerId>,
```

### `#[required]`

Required field that must be set before calling `build()`.

## Generated Methods

| Method | Description |
|--------|-------------|
| `new()` | Create factory with defaults |
| `with_<entity>(&Entity)` | Set FK from entity reference |
| `with_<field>_id(Id)` | Set FK ID directly |
| `with_<field>(value)` | Set field value |
| `build()` | Build entity in-memory (panics if required fields missing) |
| `build_with_fks(pool)` | Build entity, auto-creating FK dependencies |

## The Sentinel Trait

The `Sentinel` trait (from `factory-m8`) detects "unset" values that trigger auto-creation:

```rust
use factory_m8::Sentinel;

#[derive(Clone, Copy, Default)]
pub struct UserId(pub i64);

impl Sentinel for UserId {
    fn sentinel() -> Self { UserId(0) }
    fn is_sentinel(&self) -> bool { self.0 == 0 }
}
```

## Database Backends

Works with any database - just implement `FactoryCreate<YourPool>`:

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

    // MongoDB - set manually
    #[fk(AuditLog, "id", AuditLogFactory, no_default)]
    pub audit_log_id: Option<AuditLogId>,
}
```

## License

MIT License - see [LICENSE](LICENSE) for details.
