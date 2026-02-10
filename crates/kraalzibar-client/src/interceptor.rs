use tonic::service::Interceptor;

#[derive(Debug, Clone)]
pub struct ApiKeyInterceptor {
    api_key: Option<String>,
}

impl ApiKeyInterceptor {
    pub fn new(api_key: Option<String>) -> Self {
        Self { api_key }
    }
}

impl Interceptor for ApiKeyInterceptor {
    fn call(
        &mut self,
        mut request: tonic::Request<()>,
    ) -> Result<tonic::Request<()>, tonic::Status> {
        if let Some(ref key) = self.api_key {
            let value = format!("Bearer {key}")
                .parse()
                .map_err(|_| tonic::Status::internal("invalid api key format"))?;
            request.metadata_mut().insert("authorization", value);
        }
        Ok(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_key_interceptor_adds_authorization_header() {
        let mut interceptor = ApiKeyInterceptor::new(Some("test-key-123".to_string()));
        let request = tonic::Request::new(());

        let result = interceptor.call(request).unwrap();
        let auth = result.metadata().get("authorization").unwrap();

        assert_eq!(auth.to_str().unwrap(), "Bearer test-key-123");
    }

    #[test]
    fn api_key_interceptor_skips_when_no_key() {
        let mut interceptor = ApiKeyInterceptor::new(None);
        let request = tonic::Request::new(());

        let result = interceptor.call(request).unwrap();

        assert!(result.metadata().get("authorization").is_none());
    }
}
