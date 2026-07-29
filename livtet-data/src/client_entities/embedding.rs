//! Local vector embeddings for edition similarity search.
//!
//! Provides "find similar", "next read", and "I'm feeling lucky" features
//! powered by local embeddings computed from edition metadata.
//!
//! ## Embedding Text Composition
//!
//! Each edition's embedding is computed from:
//! - Work title and description
//! - Author names
//! - Tags, genres, subjects
//! - Series info
//! - Format (ebook, audiobook, etc.)
//!
//! ## User Taste Centroid
//!
//! For "next read" recommendations, we compute a centroid vector from
//! all editions the user has read and marked as liked/completed.

use livtet_types::DbId;
use rand::RngExt;
use sea_orm::DbConn;
#[cfg(feature = "client")]
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};

#[cfg(feature = "client")]
use crate::client_entities::edition_embedding::Entity as EditionEmbeddingEntity;

pub const MIN_LIBRARY_SIZE_FOR_RECOMMENDATIONS: usize = 100;

#[derive(Debug, Clone)]
pub struct SimilarEdition {
    pub edition_id: DbId,
    pub score: f32,
}

#[derive(Debug, Clone)]
pub struct EmbeddingResult {
    pub dimensions: usize,
    pub vector: Vec<f32>,
}

#[cfg(feature = "client")]
pub fn compose_embedding_text(
    title: &str,
    description: Option<&str>,
    authors: &[String],
    tags: &[String],
    genres: &[String],
    subjects: &[String],
    series_name: Option<&str>,
    series_position: Option<i32>,
    format: &str,
) -> String {
    let mut parts = vec![title.to_string()];

    if let Some(desc) = description
        && !desc.is_empty()
    {
        parts.push(desc.to_string());
    }

    for author in authors {
        parts.push(author.clone());
    }

    for tag in tags {
        parts.push(tag.clone());
    }

    for genre in genres {
        parts.push(genre.clone());
    }

    for subject in subjects {
        parts.push(subject.clone());
    }

    if let Some(series) = series_name {
        parts.push(format!("series:{}", series));
        if let Some(pos) = series_position {
            parts.push(format!("book {} in series", pos));
        }
    }

    parts.push(format!("format:{}", format));

    parts.join(" | ")
}

#[cfg(feature = "client")]
pub async fn compute_embedding(_text: &str) -> EmbeddingResult {
    EmbeddingResult {
        dimensions: 384,
        vector: vec![0.0; 384],
    }
}

#[cfg(feature = "client")]
pub async fn find_similar_editions(
    db: &DbConn,
    edition_id: &DbId,
    limit: usize,
) -> Result<Vec<SimilarEdition>, Box<dyn std::error::Error + Send + Sync>> {
    let embedding = EditionEmbeddingEntity::find()
        .filter(crate::client_entities::edition_embedding::Column::EditionId.eq(*edition_id))
        .one(db)
        .await?
        .ok_or("Edition embedding not found")?;

    let query_vector: Vec<f32> = bytes_to_f32_vector(&embedding.vector);

    let all_embeddings = EditionEmbeddingEntity::find().all(db).await?;

    let mut similarities: Vec<SimilarEdition> = all_embeddings
        .into_iter()
        .filter(|e| e.edition_id != *edition_id)
        .filter_map(|e| {
            let other_vector = bytes_to_f32_vector(&e.vector);
            let score = cosine_similarity(&query_vector, &other_vector)?;
            Some(SimilarEdition {
                edition_id: e.edition_id,
                score,
            })
        })
        .collect();

    similarities.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    similarities.truncate(limit);

    Ok(similarities)
}

#[cfg(feature = "client")]
pub async fn find_similar_to_taste(
    db: &DbConn,
    read_edition_ids: &[DbId],
    candidate_edition_ids: &[DbId],
    limit: usize,
) -> Result<Vec<SimilarEdition>, Box<dyn std::error::Error + Send + Sync>> {
    if read_edition_ids.is_empty() {
        return Ok(vec![]);
    }

    let read_embeddings = EditionEmbeddingEntity::find()
        .filter(
            crate::client_entities::edition_embedding::Column::EditionId
                .is_in(read_edition_ids.to_vec()),
        )
        .all(db)
        .await?;

    if read_embeddings.is_empty() {
        return Ok(vec![]);
    }

    let centroid = compute_centroid(
        &read_embeddings
            .iter()
            .map(|e| bytes_to_f32_vector(&e.vector))
            .collect::<Vec<_>>(),
    );

    let candidate_embeddings = EditionEmbeddingEntity::find()
        .filter(
            crate::client_entities::edition_embedding::Column::EditionId
                .is_in(candidate_edition_ids.to_vec()),
        )
        .all(db)
        .await?;

    let mut similarities: Vec<SimilarEdition> = candidate_embeddings
        .into_iter()
        .filter_map(|e| {
            let other_vector = bytes_to_f32_vector(&e.vector);
            let score = cosine_similarity(&centroid, &other_vector)?;
            Some(SimilarEdition {
                edition_id: e.edition_id,
                score,
            })
        })
        .collect();

    similarities.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    similarities.truncate(limit);

    Ok(similarities)
}

#[cfg(feature = "client")]
pub async fn feeling_lucky(
    db: &DbConn,
    read_edition_ids: &[DbId],
    limit: usize,
) -> Result<Vec<DbId>, Box<dyn std::error::Error + Send + Sync>> {
    let similar = find_similar_to_taste(db, read_edition_ids, read_edition_ids, limit * 2).await?;

    let mut rng = rand::rng();

    let mut result: Vec<DbId> = similar.into_iter().map(|s| s.edition_id).collect();
    for i in (1..result.len()).rev() {
        let j = rng.random_range(0..=i);
        result.swap(i, j);
    }
    result.truncate(limit);

    Ok(result)
}

fn bytes_to_f32_vector(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

pub fn f32_vector_to_bytes(vector: &[f32]) -> Vec<u8> {
    vector
        .iter()
        .flat_map(|f| f.to_le_bytes().to_vec())
        .collect()
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> Option<f32> {
    if a.len() != b.len() || a.is_empty() {
        return None;
    }

    let dot_product: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let magnitude_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let magnitude_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

    if magnitude_a == 0.0 || magnitude_b == 0.0 {
        return None;
    }

    Some(dot_product / (magnitude_a * magnitude_b))
}

fn compute_centroid(vectors: &[Vec<f32>]) -> Vec<f32> {
    if vectors.is_empty() {
        return vec![];
    }

    let dim = vectors[0].len();
    let mut centroid = vec![0.0; dim];

    for vector in vectors {
        for (i, val) in vector.iter().enumerate() {
            centroid[i] += val;
        }
    }

    let count = vectors.len() as f32;
    for val in &mut centroid {
        *val /= count;
    }

    centroid
}

#[cfg(feature = "client")]
pub async fn is_recommendations_enabled(
    db: &DbConn,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    let count = EditionEmbeddingEntity::find().count(db).await?;
    Ok(count as usize >= MIN_LIBRARY_SIZE_FOR_RECOMMENDATIONS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        assert!((cosine_similarity(&a, &b).unwrap() - 1.0).abs() < 0.001);

        let c = vec![0.0, 1.0, 0.0];
        assert!((cosine_similarity(&a, &c).unwrap() - 0.0).abs() < 0.001);

        let d = vec![-1.0, 0.0, 0.0];
        assert!((cosine_similarity(&a, &d).unwrap() - (-1.0)).abs() < 0.001);
    }

    #[test]
    fn test_bytes_conversion() {
        let original = vec![1.0, 2.0, 3.0, 4.0];
        let bytes = f32_vector_to_bytes(&original);
        let recovered = bytes_to_f32_vector(&bytes);
        assert_eq!(original.len(), recovered.len());
        for (o, r) in original.iter().zip(recovered.iter()) {
            assert!((o - r).abs() < 0.0001);
        }
    }

    #[test]
    fn test_compose_embedding_text() {
        let text = compose_embedding_text(
            "The Great Gatsby",
            Some("A classic American novel"),
            &["F. Scott Fitzgerald".to_string()],
            &["classic".to_string()],
            &["literary fiction".to_string()],
            &["1920s".to_string()],
            None,
            None,
            "ebook",
        );
        assert!(text.contains("The Great Gatsby"));
        assert!(text.contains("F. Scott Fitzgerald"));
        assert!(text.contains("format:ebook"));
    }
}
