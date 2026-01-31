//! Integration tests for Factory derive macro with actual database operations.
//!
//! These tests verify that build_with_fks() correctly auto-creates FK dependencies.

use async_trait::async_trait;
use factory_m8::{FactoryCreate, Sentinel};
use factory_derive::Factory;
use sqlx::PgPool;
use std::error::Error;

// =============================================================================
// SIMPLE ID TYPES (demonstrating Sentinel trait usage without typed-ids)
// =============================================================================

macro_rules! define_simple_id {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Default, sqlx::Type)]
        #[sqlx(transparent)]
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

define_simple_id!(PersonId);
define_simple_id!(NoteId);
define_simple_id!(TestId);

// =============================================================================
// ENTITIES
// =============================================================================

/// Simple entity with required and optional fields
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct Person {
    pub id: PersonId,
    pub first_name: String,
    pub last_name: Option<String>,
}

/// Entity that references Person (required FK)
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct Note {
    pub id: NoteId,
    pub person_id: PersonId, // Required FK to Person
    pub content: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct MaybePersonNoteMappingEntity {
    pub id: TestId,
    pub person_id: Option<PersonId>,
    pub note_id: Option<NoteId>,
}

// =============================================================================
// FACTORIES
// =============================================================================

#[derive(Debug, Factory)]
#[factory(entity = Person)]
pub struct PersonFactory {
    #[pk]
    pub id: PersonId,

    #[required]
    pub first_name: Option<String>,

    pub last_name: Option<String>,
}

// Custom Default to provide sensible defaults for required fields
impl Default for PersonFactory {
    fn default() -> Self {
        Self {
            id: PersonId::sentinel(),
            first_name: Some("Auto-Generated".to_string()), // Default for auto-creation
            last_name: None,
        }
    }
}

#[async_trait]
impl FactoryCreate<PgPool> for PersonFactory {
    type Entity = Person;

    async fn create(self, pool: &PgPool) -> Result<Person, Box<dyn Error + Send + Sync>> {
        let entity = self.build_with_fks(pool).await?;

        let person = sqlx::query_as::<_, Person>(
            "INSERT INTO person (first_name, last_name) VALUES ($1, $2) RETURNING *",
        )
        .bind(&entity.first_name)
        .bind(&entity.last_name)
        .fetch_one(pool)
        .await?;

        Ok(person)
    }
}

#[derive(Debug, Factory)]
#[factory(entity = Note)]
pub struct NoteFactory {
    #[pk]
    pub id: i64,

    #[fk(Person, "id", PersonFactory)]
    pub person_id: PersonId,

    #[required]
    pub content: Option<String>,
}

// Custom Default to provide sensible defaults for required fields
impl Default for NoteFactory {
    fn default() -> Self {
        Self {
            id: 0,
            person_id: PersonId::sentinel(),
            content: Some("Default note content".to_string()),
        }
    }
}

#[async_trait]
impl FactoryCreate<PgPool> for NoteFactory {
    type Entity = Note;

    async fn create(self, pool: &PgPool) -> Result<Note, Box<dyn Error + Send + Sync>> {
        let entity = self.build_with_fks(pool).await?;

        let note = sqlx::query_as::<_, Note>(
            "INSERT INTO note (person_id, content) VALUES ($1, $2) RETURNING *",
        )
        .bind(entity.person_id)
        .bind(&entity.content)
        .fetch_one(pool)
        .await?;

        Ok(note)
    }
}

#[derive(Debug, Factory)]
#[factory(entity = MaybePersonNoteMappingEntity)]
pub struct MaybePersonNoteMappingEntityFactory {
    #[pk]
    pub id: TestId,

    #[fk(Person, "id", PersonFactory)]
    pub person_id: Option<PersonId>,

    #[fk(Note, "id", NoteFactory, no_default)]
    pub note_id: Option<NoteId>,
}

impl Default for MaybePersonNoteMappingEntityFactory {
    fn default() -> Self {
        Self {
            id: TestId::sentinel(),
            person_id: None,
            note_id: None,
        }
    }
}

#[async_trait]
impl FactoryCreate<PgPool> for MaybePersonNoteMappingEntityFactory {
    type Entity = MaybePersonNoteMappingEntity;

    async fn create(
        self,
        pool: &PgPool,
    ) -> Result<MaybePersonNoteMappingEntity, Box<dyn Error + Send + Sync>> {
        let entity = self.build_with_fks(pool).await?;

        let model = sqlx::query_as::<_, MaybePersonNoteMappingEntity>(
            "INSERT INTO person_note_mapping (person_id, note_id) values ($1, $2) RETURNING *",
        )
        .bind(entity.person_id)
        .bind(entity.note_id)
        .fetch_one(pool)
        .await?;

        Ok(model)
    }
}

// =============================================================================
// HELPER: Create tables for tests
// =============================================================================

async fn setup_tables(pool: &PgPool) -> Result<(), sqlx::Error> {
    let statements = vec![
        r#"
        CREATE TABLE IF NOT EXISTS person (
            id BIGSERIAL PRIMARY KEY,
            first_name TEXT NOT NULL,
            last_name TEXT
        )
        "#,
        r#"
        CREATE TABLE IF NOT EXISTS note (
            id BIGSERIAL PRIMARY KEY,
            person_id BIGINT NOT NULL REFERENCES person(id),
            content TEXT NOT NULL
        )
        "#,
        r#"
        CREATE TABLE IF NOT EXISTS person_note_mapping (
            id BIGSERIAL PRIMARY KEY,
            person_id BIGINT NULL REFERENCES person(id),
            note_id BIGINT NULL REFERENCES note(id)
        )
        "#,
        "truncate person_note_mapping cascade",
        "truncate person cascade",
        "truncate note cascade",
    ];

    for s in statements {
        sqlx::query(s).execute(pool).await?;
    }

    Ok(())
}

// =============================================================================
// TESTS
// =============================================================================

/// Test that NoteFactory auto-creates Person when person_id is not set.
///
/// This is the key test - we call NoteFactory::new().create() WITHOUT setting
/// person_id, and verify that:
/// 1. A Person record was auto-created
/// 2. A Note record was created with the correct person_id FK
#[sqlx::test]
async fn test_factory_auto_creates_fk_dependency(
    pool: PgPool,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    setup_tables(&pool).await?;

    // Verify tables are empty
    let person_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM person")
        .fetch_one(&pool)
        .await?;
    assert_eq!(person_count.0, 0, "person table should be empty initially");

    let note_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM note")
        .fetch_one(&pool)
        .await?;
    assert_eq!(note_count.0, 0, "note table should be empty initially");

    // Create note WITHOUT setting person_id - should auto-create person!
    let note = NoteFactory::new()
        .with_content("This is a test note")
        .create(&pool)
        .await?;

    // Verify note was created
    assert!(note.id.0 > 0, "Note should have a valid ID");
    assert_eq!(note.content, "This is a test note");
    assert!(note.person_id.0 > 0, "Note should have a person_id");

    // Verify person was auto-created
    let person_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM person")
        .fetch_one(&pool)
        .await?;
    assert_eq!(person_count.0, 1, "Person should have been auto-created");

    // Verify the person exists and note references it
    let person: Person = sqlx::query_as("SELECT * FROM person WHERE id = $1")
        .bind(note.person_id)
        .fetch_one(&pool)
        .await?;

    assert_eq!(
        person.id, note.person_id,
        "Note should reference the created person"
    );

    println!(
        "SUCCESS: Auto-created Person (id={:?}) for Note (id={:?})",
        person.id, note.id
    );

    Ok(())
}

/// Test that we can explicitly set the FK to use an existing entity.
#[sqlx::test]
async fn test_factory_uses_explicit_fk(pool: PgPool) -> Result<(), Box<dyn Error + Send + Sync>> {
    setup_tables(&pool).await?;

    // First create a person explicitly
    let person = PersonFactory::new()
        .with_first_name("John")
        .with_last_name("Doe")
        .create(&pool)
        .await?;

    assert_eq!(person.first_name, "John");
    assert_eq!(person.last_name, Some("Doe".to_string()));

    // Create note WITH explicit person reference
    let note = NoteFactory::new()
        .with_person(&person) // Use the existing person
        .with_content("Note for John")
        .create(&pool)
        .await?;

    // Verify note references the person we created
    assert_eq!(note.person_id, person.id);

    // Verify only 1 person exists (no auto-creation)
    let person_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM person")
        .fetch_one(&pool)
        .await?;
    assert_eq!(
        person_count.0, 1,
        "Should only have 1 person (no auto-creation)"
    );

    println!(
        "SUCCESS: Note (id={:?}) correctly references existing Person (id={:?})",
        note.id, person.id
    );

    Ok(())
}

/// Test that we can set FK by ID directly.
#[sqlx::test]
async fn test_factory_uses_explicit_fk_id(
    pool: PgPool,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    setup_tables(&pool).await?;

    // First create a person
    let person = PersonFactory::new()
        .with_first_name("Jane")
        .create(&pool)
        .await?;

    // Create note with explicit person_id
    let note = NoteFactory::new()
        .with_person_id(person.id) // Use ID directly
        .with_content("Note for Jane")
        .create(&pool)
        .await?;

    assert_eq!(note.person_id, person.id);

    // Verify only 1 person
    let person_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM person")
        .fetch_one(&pool)
        .await?;
    assert_eq!(person_count.0, 1);

    Ok(())
}

/// Test creating multiple notes for the same person.
#[sqlx::test]
async fn test_multiple_notes_same_person(pool: PgPool) -> Result<(), Box<dyn Error + Send + Sync>> {
    setup_tables(&pool).await?;

    let person = PersonFactory::new()
        .with_first_name("Bob")
        .create(&pool)
        .await?;

    let note1 = NoteFactory::new()
        .with_person(&person)
        .with_content("First note")
        .create(&pool)
        .await?;

    let note2 = NoteFactory::new()
        .with_person(&person)
        .with_content("Second note")
        .create(&pool)
        .await?;

    assert_eq!(note1.person_id, person.id);
    assert_eq!(note2.person_id, person.id);
    assert_ne!(note1.id, note2.id);

    let note_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM note WHERE person_id = $1")
        .bind(person.id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(note_count.0, 2);

    Ok(())
}

/// Test creating multiple notes without specifying the person at all
#[sqlx::test]
async fn test_multiple_notes_no_person(pool: PgPool) -> Result<(), Box<dyn Error + Send + Sync>> {
    setup_tables(&pool).await?;

    let _ = NoteFactory::new()
        .with_content("First Note")
        .create(&pool)
        .await?;

    let _ = NoteFactory::new()
        .with_content("Second Note")
        .create(&pool)
        .await?;

    let person_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM person")
        .fetch_one(&pool)
        .await?;
    assert_eq!(person_count.0, 2);

    Ok(())
}

#[sqlx::test]
async fn test_no_default_flag(pool: PgPool) -> Result<(), Box<dyn Error + Send + Sync>> {
    setup_tables(&pool).await?;

    let mapping = MaybePersonNoteMappingEntityFactory::new()
        .create(&pool)
        .await?;

    assert!(mapping.person_id.is_some());
    assert!(mapping.note_id.is_none());

    dbg!(mapping);

    let person_count: (i64,) = sqlx::query_as("SELECT COUNT(*) from person")
        .fetch_one(&pool)
        .await?;
    let notes_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM  note")
        .fetch_one(&pool)
        .await?;

    assert_eq!(
        notes_count.0, 0,
        "Note should not be created. [NOTE]: PLEASE BE CAREFUL AS NOTE CREATES PERSON AS WELL"
    );
    assert_eq!(person_count.0, 1, "Person should be created");

    Ok(())
}
