use tonic::service::Interceptor;
use tonic::{Request, Status};

const AUTH_HEADER: &str = "authorization";

#[derive(Clone, Debug)]
pub struct AuthInterceptor {
    expected_token: Option<String>,
}

impl AuthInterceptor {
    pub fn from_env_var(var: &str) -> Result<Self, String> {
        match std::env::var(var) {
            Ok(token) => {
                let trimmed = token.trim();
                if trimmed.is_empty() {
                    return Err(format!("{var} must not be empty when set"));
                }
                Ok(Self {
                    expected_token: Some(trimmed.to_string()),
                })
            }
            Err(std::env::VarError::NotPresent) => Ok(Self {
                expected_token: None,
            }),
            Err(std::env::VarError::NotUnicode(_)) => Err(format!("{var} must be valid UTF-8")),
        }
    }

    pub fn required(token: &str) -> Self {
        Self {
            expected_token: Some(token.to_string()),
        }
    }

    fn check_request(&self, request: Request<()>) -> Result<Request<()>, Status> {
        let Some(expected) = self.expected_token.as_ref() else {
            return Ok(request);
        };

        let raw_header = request
            .metadata()
            .get(AUTH_HEADER)
            .ok_or_else(|| Status::unauthenticated("missing authorization metadata"))?;

        let raw = raw_header
            .to_str()
            .map_err(|_| Status::unauthenticated("authorization metadata is not valid ASCII"))?;

        let (scheme, token) = raw
            .split_once(' ')
            .ok_or_else(|| Status::unauthenticated("invalid authorization metadata format"))?;

        if !scheme.eq_ignore_ascii_case("bearer") {
            return Err(Status::unauthenticated(
                "authorization scheme must be Bearer",
            ));
        }
        if token != expected {
            return Err(Status::unauthenticated("invalid bearer token"));
        }

        Ok(request)
    }
}

impl Interceptor for AuthInterceptor {
    fn call(&mut self, request: Request<()>) -> Result<Request<()>, Status> {
        self.check_request(request)
    }
}
