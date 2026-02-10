use std::fmt;

#[derive(Debug)]
pub enum ClientError {
    Connection(String),
    InvalidArgument(String),
    NotFound(String),
    PermissionDenied(String),
    FailedPrecondition(String),
    Internal(String),
    Timeout,
    Status(tonic::Status),
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClientError::Connection(msg) => write!(f, "connection error: {msg}"),
            ClientError::InvalidArgument(msg) => write!(f, "invalid argument: {msg}"),
            ClientError::NotFound(msg) => write!(f, "not found: {msg}"),
            ClientError::PermissionDenied(msg) => write!(f, "permission denied: {msg}"),
            ClientError::FailedPrecondition(msg) => write!(f, "failed precondition: {msg}"),
            ClientError::Internal(msg) => write!(f, "internal error: {msg}"),
            ClientError::Timeout => write!(f, "request timed out"),
            ClientError::Status(status) => write!(f, "grpc status: {status}"),
        }
    }
}

impl std::error::Error for ClientError {}

impl From<tonic::Status> for ClientError {
    fn from(status: tonic::Status) -> Self {
        match status.code() {
            tonic::Code::NotFound => ClientError::NotFound(status.message().to_string()),
            tonic::Code::PermissionDenied => {
                ClientError::PermissionDenied(status.message().to_string())
            }
            tonic::Code::InvalidArgument => {
                ClientError::InvalidArgument(status.message().to_string())
            }
            tonic::Code::DeadlineExceeded => ClientError::Timeout,
            tonic::Code::Internal => ClientError::Internal(status.message().to_string()),
            tonic::Code::FailedPrecondition => {
                ClientError::FailedPrecondition(status.message().to_string())
            }
            tonic::Code::Unavailable => ClientError::Connection(status.message().to_string()),
            _ => ClientError::Status(status),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_error_from_tonic_status_not_found() {
        let status = tonic::Status::not_found("resource missing");
        let err: ClientError = status.into();

        assert!(matches!(err, ClientError::NotFound(msg) if msg == "resource missing"));
    }

    #[test]
    fn client_error_from_tonic_status_permission_denied() {
        let status = tonic::Status::permission_denied("no access");
        let err: ClientError = status.into();

        assert!(matches!(err, ClientError::PermissionDenied(msg) if msg == "no access"));
    }

    #[test]
    fn client_error_from_tonic_status_invalid_argument() {
        let status = tonic::Status::invalid_argument("bad input");
        let err: ClientError = status.into();

        assert!(matches!(err, ClientError::InvalidArgument(msg) if msg == "bad input"));
    }

    #[test]
    fn client_error_from_tonic_status_deadline_exceeded() {
        let status = tonic::Status::deadline_exceeded("too slow");
        let err: ClientError = status.into();

        assert!(matches!(err, ClientError::Timeout));
    }

    #[test]
    fn client_error_from_tonic_status_internal() {
        let status = tonic::Status::internal("server broke");
        let err: ClientError = status.into();

        assert!(matches!(err, ClientError::Internal(msg) if msg == "server broke"));
    }

    #[test]
    fn client_error_display_includes_message() {
        let err = ClientError::NotFound("doc not found".to_string());
        assert_eq!(err.to_string(), "not found: doc not found");

        let err = ClientError::Timeout;
        assert_eq!(err.to_string(), "request timed out");

        let err = ClientError::Connection("refused".to_string());
        assert_eq!(err.to_string(), "connection error: refused");
    }
}
