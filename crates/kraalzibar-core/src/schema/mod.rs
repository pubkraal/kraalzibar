mod parser;
pub mod types;
pub mod validation;

pub use parser::{ParseError, parse_schema};
pub use validation::{
    BreakingChange, SchemaLimits, ValidationError, detect_breaking_changes, validate_schema_limits,
};
