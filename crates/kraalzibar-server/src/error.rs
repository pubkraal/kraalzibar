use kraalzibar_core::engine::CheckError;
use kraalzibar_core::schema::{ParseError, ValidationError};
use kraalzibar_storage::StorageError;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("check error: {0}")]
    Check(#[from] CheckError),

    #[error("storage error: {0}")]
    Storage(#[from] StorageError),

    #[error("parse error: {0}")]
    Parse(#[from] ParseError),

    #[error("schema validation errors: {}", format_validation_errors(.0))]
    Validation(Vec<ValidationError>),

    #[error("breaking schema changes detected (use force=true to override)")]
    BreakingChanges(Vec<kraalzibar_core::schema::BreakingChange>),

    #[error("schema not found")]
    SchemaNotFound,
}

fn format_validation_errors(errors: &[ValidationError]) -> String {
    errors
        .iter()
        .map(|e| e.to_string())
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_error_from_check_error() {
        let check_err = CheckError::TypeNotFound("document".to_string());
        let api_err: ApiError = check_err.into();

        assert!(
            api_err.to_string().contains("document"),
            "expected 'document' in error message, got: {api_err}"
        );
    }

    #[test]
    fn api_error_from_storage_error() {
        let storage_err = StorageError::EmptyDeleteFilter;
        let api_err: ApiError = storage_err.into();

        assert!(
            api_err.to_string().contains("delete filter"),
            "expected 'delete filter' in error message, got: {api_err}"
        );
    }

    #[test]
    fn api_error_from_parse_error() {
        let parse_err = ParseError::MixedOperators;
        let api_err: ApiError = parse_err.into();

        assert!(
            api_err.to_string().contains("mixed operators"),
            "expected 'mixed operators' in error message, got: {api_err}"
        );
    }

    #[test]
    fn api_error_validation_formats_multiple_errors() {
        let errors = vec![
            ValidationError::TooManyTypes {
                count: 60,
                limit: 50,
            },
            ValidationError::TooManyRelations {
                type_name: "doc".to_string(),
                count: 40,
                limit: 30,
            },
        ];
        let api_err = ApiError::Validation(errors);
        let msg = api_err.to_string();

        assert!(msg.contains("60"), "should contain count 60: {msg}");
        assert!(msg.contains("doc"), "should contain type name: {msg}");
    }

    #[test]
    fn api_error_schema_not_found() {
        let api_err = ApiError::SchemaNotFound;
        assert!(api_err.to_string().contains("schema not found"));
    }
}
