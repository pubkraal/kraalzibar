pub const MAX_BATCH_SIZE: usize = 1000;
pub const MAX_IDENTIFIER_LENGTH: usize = 256;

#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("{field} must not be empty")]
    EmptyIdentifier { field: String },

    #[error("{field} exceeds maximum length of {MAX_IDENTIFIER_LENGTH}")]
    IdentifierTooLong { field: String },

    #[error("batch size {size} exceeds maximum of {MAX_BATCH_SIZE}")]
    BatchTooLarge { size: usize },
}

pub fn validate_identifier(field: &str, value: &str) -> Result<(), ValidationError> {
    if value.is_empty() {
        return Err(ValidationError::EmptyIdentifier {
            field: field.to_string(),
        });
    }

    if value.len() > MAX_IDENTIFIER_LENGTH {
        return Err(ValidationError::IdentifierTooLong {
            field: field.to_string(),
        });
    }

    Ok(())
}

pub fn validate_batch_size(size: usize) -> Result<(), ValidationError> {
    if size > MAX_BATCH_SIZE {
        return Err(ValidationError::BatchTooLarge { size });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_identifier_accepts_valid_string() {
        let result = validate_identifier("object_type", "document");

        assert!(result.is_ok());
    }

    #[test]
    fn validate_identifier_rejects_empty_string() {
        let result = validate_identifier("object_type", "");

        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("object_type"),
            "expected field name in error, got: {msg}"
        );
        assert!(
            msg.contains("must not be empty"),
            "expected empty message, got: {msg}"
        );
    }

    #[test]
    fn validate_identifier_rejects_too_long_string() {
        let long = "a".repeat(MAX_IDENTIFIER_LENGTH + 1);
        let result = validate_identifier("object_id", &long);

        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("object_id"),
            "expected field name in error, got: {msg}"
        );
        assert!(
            msg.contains("maximum length"),
            "expected length message, got: {msg}"
        );
    }

    #[test]
    fn validate_identifier_accepts_max_length_string() {
        let exact = "a".repeat(MAX_IDENTIFIER_LENGTH);
        let result = validate_identifier("relation", &exact);

        assert!(result.is_ok());
    }

    #[test]
    fn validate_batch_size_accepts_within_limit() {
        let result = validate_batch_size(MAX_BATCH_SIZE);

        assert!(result.is_ok());
    }

    #[test]
    fn validate_batch_size_rejects_over_limit() {
        let result = validate_batch_size(MAX_BATCH_SIZE + 1);

        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("batch size"),
            "expected batch size in error, got: {msg}"
        );
        assert!(
            msg.contains("1001"),
            "expected size value in error, got: {msg}"
        );
    }

    #[test]
    fn validate_batch_size_accepts_zero() {
        let result = validate_batch_size(0);

        assert!(result.is_ok());
    }
}
