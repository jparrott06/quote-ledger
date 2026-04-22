//! Composite gRPC interceptor: optional process-wide request rate limit, then auth.

use std::num::NonZeroU32;
use std::sync::Arc;

use governor::clock::DefaultClock;
use governor::state::{InMemoryState, NotKeyed};
use governor::{Quota, RateLimiter};
use metrics::counter;
use tonic::service::Interceptor;
use tonic::{Request, Status};

use crate::auth::AuthInterceptor;

/// Process-wide limit on accepted **RPCs** (all methods share one token bucket).
pub type GlobalRequestLimiter = RateLimiter<NotKeyed, InMemoryState, DefaultClock>;

#[derive(Clone)]
pub struct LedgerGrpcInterceptor {
    auth: AuthInterceptor,
    limiter: Option<Arc<GlobalRequestLimiter>>,
}

impl LedgerGrpcInterceptor {
    pub fn new(auth: AuthInterceptor, requests_per_second: Option<NonZeroU32>) -> Self {
        let limiter = requests_per_second
            .map(|n| Arc::new(GlobalRequestLimiter::direct(Quota::per_second(n))));
        Self { auth, limiter }
    }
}

impl Interceptor for LedgerGrpcInterceptor {
    fn call(&mut self, request: Request<()>) -> Result<Request<()>, Status> {
        if let Some(lim) = &self.limiter {
            if lim.check().is_err() {
                counter!("quote_ledger_grpc_rate_limited_total").increment(1);
                return Err(Status::resource_exhausted(
                    "global gRPC request rate limit exceeded (set GRPC_RATE_LIMIT_RPS)",
                ));
            }
        }
        self.auth.intercept(request)
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;
    use std::time::Duration;

    use governor::{Quota, RateLimiter};
    use tonic::Request;

    use super::*;

    #[test]
    fn burst_one_then_immediate_second_is_denied() {
        let quota = Quota::with_period(Duration::from_secs(3600))
            .expect("quota")
            .allow_burst(NonZeroU32::new(1).unwrap());
        let lim: Arc<GlobalRequestLimiter> = Arc::new(RateLimiter::direct(quota));
        assert!(lim.check().is_ok());
        assert!(lim.check().is_err());
    }

    #[test]
    fn interceptor_returns_err_when_bucket_empty() {
        let auth =
            AuthInterceptor::from_env_var("QUOTE_LEDGER_AUTH_TOKEN_UNUSED_FOR_RATE_LIMIT_TEST")
                .expect("env");
        let quota = Quota::with_period(Duration::from_secs(3600))
            .expect("quota")
            .allow_burst(NonZeroU32::new(1).unwrap());
        let lim: Arc<GlobalRequestLimiter> = Arc::new(RateLimiter::direct(quota));
        let mut ix = LedgerGrpcInterceptor {
            auth,
            limiter: Some(lim),
        };
        assert!(ix.call(Request::new(())).is_ok());
        assert!(ix.call(Request::new(())).is_err());
    }
}
