use folio_core::ports::EmbeddingBatch;

#[test]
fn missing_vectors_cannot_complete_a_batch() {
    let batch = EmbeddingBatch {
        vectors: vec![],
        model: "test".into(),
        dimensions: 2,
    };
    assert!(batch.validate(1).is_err());
}

#[test]
fn extra_vectors_cannot_be_silently_discarded() {
    let batch = EmbeddingBatch {
        vectors: vec![vec![1.0, 0.0], vec![0.0, 1.0]],
        model: "test".into(),
        dimensions: 2,
    };
    assert!(batch.validate(1).is_err());
}

#[test]
fn invalid_dimensions_cannot_complete_a_batch() {
    let batch = EmbeddingBatch {
        vectors: vec![vec![1.0]],
        model: "test".into(),
        dimensions: 2,
    };
    assert!(batch.validate(1).is_err());
}

#[test]
fn nonfinite_vectors_cannot_complete_a_batch() {
    let batch = EmbeddingBatch {
        vectors: vec![vec![f32::NAN, 0.0]],
        model: "test".into(),
        dimensions: 2,
    };
    assert!(batch.validate(1).is_err());
}

#[test]
fn complete_vectors_pass_validation() {
    let batch = EmbeddingBatch {
        vectors: vec![vec![1.0, 0.0]],
        model: "test".into(),
        dimensions: 2,
    };
    assert!(batch.validate(1).is_ok());
}
