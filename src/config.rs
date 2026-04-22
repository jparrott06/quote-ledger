//! Centralized service configuration from environment variables.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::time::Duration;

use crate::ledger::ReliabilityLimits;

const KNOWN_ENV_KEYS: &[&str] = &[
    "APPEND_IDLE_TIMEOUT_MS",
    "APPEND_MAX_COMMANDS",
    "GRPC_CONCURRENCY_LIMIT",
    "GRPC_KEEPALIVE_INTERVAL_MS",
    "GRPC_KEEPALIVE_TIMEOUT_MS",
    "GRPC_LISTEN_ADDR",
    "GRPC_RATE_LIMIT_RPS",
    "GRPC_TLS_CERT",
    "GRPC_TLS_KEY",
    "METRICS_ADDR",
    "QUOTE_LEDGER_AUTH_TOKEN",
    "QUOTE_LEDGER_DB",
    "QUOTE_LEDGER_REFLECTION",
    "STRICT_CONFIG",
];

fn env_u64(name: &str, default: u64) -> Result<u64, String> {
    match std::env::var(name) {
        Ok(v) => v.parse::<u64>().map_err(|e| format!("{name}: {e}")),
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name}: must be valid UTF-8")),
    }
}

fn env_usize(name: &str, default: usize) -> Result<usize, String> {
    match std::env::var(name) {
        Ok(v) => v.parse::<usize>().map_err(|e| format!("{name}: {e}")),
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name}: must be valid UTF-8")),
    }
}

fn env_bool(name: &str, default: bool) -> Result<bool, String> {
    match std::env::var(name) {
        Ok(v) => match v.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Ok(true),
            "0" | "false" | "no" | "off" => Ok(false),
            other => Err(format!("{name}: expected true/false, got {other:?}")),
        },
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name}: must be valid UTF-8")),
    }
}

fn env_opt_nonzero_u32(name: &str) -> Result<Option<NonZeroU32>, String> {
    match std::env::var(name) {
        Ok(v) => {
            let t = v.trim();
            if t.is_empty() {
                return Err(format!("{name} must not be empty when set"));
            }
            let n: u32 = t
                .parse()
                .map_err(|e| format!("{name}: invalid integer: {e}"))?;
            let nz = NonZeroU32::new(n).ok_or_else(|| format!("{name} must be >= 1"))?;
            Ok(Some(nz))
        }
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name}: must be valid UTF-8")),
    }
}

fn env_opt_path(name: &str) -> Result<Option<PathBuf>, String> {
    match std::env::var(name) {
        Ok(v) => {
            let t = v.trim();
            if t.is_empty() {
                return Err(format!("{name} must not be empty when set"));
            }
            Ok(Some(PathBuf::from(t)))
        }
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name}: must be valid UTF-8")),
    }
}

fn validate_strict_unknown() -> Result<(), String> {
    if !env_bool("STRICT_CONFIG", false)? {
        return Ok(());
    }
    let known: HashSet<&'static str> = KNOWN_ENV_KEYS.iter().copied().collect();
    for (k, _) in std::env::vars() {
        let is_quote_ledger = k.starts_with("QUOTE_LEDGER_");
        let is_append = k.starts_with("APPEND_");
        let is_grpc = k.starts_with("GRPC_");
        if !(is_quote_ledger || is_append || is_grpc) {
            continue;
        }
        if known.contains(k.as_str()) {
            continue;
        }
        return Err(format!(
            "STRICT_CONFIG: unknown environment variable {k:?} (typo?)"
        ));
    }
    Ok(())
}

/// Fully validated runtime configuration for the `quote_ledger` binary.
#[derive(Debug, Clone)]
pub struct ServiceConfig {
    pub grpc_listen: SocketAddr,
    pub metrics_listen: SocketAddr,
    pub db_path: String,
    pub reliability: ReliabilityLimits,
    pub grpc_concurrency_limit: usize,
    pub grpc_keepalive_interval: Duration,
    pub grpc_keepalive_timeout: Duration,
    pub reflection_enabled: bool,
    pub grpc_tls_cert: Option<PathBuf>,
    pub grpc_tls_key: Option<PathBuf>,
    /// When set, all gRPC RPCs share a single process-wide token bucket (`governor`).
    pub grpc_rate_limit_rps: Option<NonZeroU32>,
}

impl ServiceConfig {
    /// Load from process environment and `argv` (first positional = gRPC listen addr).
    pub fn from_env_and_args() -> Result<Self, String> {
        validate_strict_unknown()?;

        let grpc_listen = match std::env::var("GRPC_LISTEN_ADDR") {
            Ok(v) => v.parse().map_err(|e| format!("GRPC_LISTEN_ADDR: {e}"))?,
            Err(std::env::VarError::NotPresent) => std::env::args()
                .nth(1)
                .unwrap_or_else(|| "127.0.0.1:50051".to_string())
                .parse()
                .map_err(|e| format!("grpc listen address (argv[1] or default): {e}"))?,
            Err(std::env::VarError::NotUnicode(_)) => {
                return Err("GRPC_LISTEN_ADDR: must be valid UTF-8".into());
            }
        };

        let metrics_listen: SocketAddr = std::env::var("METRICS_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:9090".into())
            .parse()
            .map_err(|e| format!("METRICS_ADDR: {e}"))?;

        let db_path = std::env::var("QUOTE_LEDGER_DB").unwrap_or_else(|_| "quote_ledger.db".into());

        let append_idle_timeout_ms = env_u64("APPEND_IDLE_TIMEOUT_MS", 10_000)?;
        let append_max_commands = env_usize("APPEND_MAX_COMMANDS", 512)?;
        if append_max_commands < 1 {
            return Err("APPEND_MAX_COMMANDS must be >= 1".into());
        }
        if !(1..=3_600_000).contains(&append_idle_timeout_ms) {
            return Err("APPEND_IDLE_TIMEOUT_MS must be between 1 and 3600000".into());
        }

        let grpc_concurrency_limit = env_usize("GRPC_CONCURRENCY_LIMIT", 128)?;
        if grpc_concurrency_limit < 1 {
            return Err("GRPC_CONCURRENCY_LIMIT must be >= 1".into());
        }

        let grpc_keepalive_interval_ms = env_u64("GRPC_KEEPALIVE_INTERVAL_MS", 30_000)?;
        let grpc_keepalive_timeout_ms = env_u64("GRPC_KEEPALIVE_TIMEOUT_MS", 10_000)?;
        if grpc_keepalive_interval_ms < 1 {
            return Err("GRPC_KEEPALIVE_INTERVAL_MS must be >= 1".into());
        }
        if grpc_keepalive_timeout_ms < 1 {
            return Err("GRPC_KEEPALIVE_TIMEOUT_MS must be >= 1".into());
        }
        if grpc_keepalive_timeout_ms >= grpc_keepalive_interval_ms {
            return Err(
                "GRPC_KEEPALIVE_TIMEOUT_MS must be less than GRPC_KEEPALIVE_INTERVAL_MS".into(),
            );
        }

        let reflection_enabled = env_bool("QUOTE_LEDGER_REFLECTION", true)?;

        let cert = env_opt_path("GRPC_TLS_CERT")?;
        let key = env_opt_path("GRPC_TLS_KEY")?;
        let (grpc_tls_cert, grpc_tls_key) = match (&cert, &key) {
            (Some(c), Some(k)) => (Some(c.clone()), Some(k.clone())),
            (None, None) => (None, None),
            _ => {
                return Err("GRPC_TLS_CERT and GRPC_TLS_KEY must both be set or both unset".into());
            }
        };

        let grpc_rate_limit_rps = env_opt_nonzero_u32("GRPC_RATE_LIMIT_RPS")?;

        Ok(Self {
            grpc_listen,
            metrics_listen,
            db_path,
            reliability: ReliabilityLimits {
                append_idle_timeout: Duration::from_millis(append_idle_timeout_ms),
                append_max_commands,
            },
            grpc_concurrency_limit,
            grpc_keepalive_interval: Duration::from_millis(grpc_keepalive_interval_ms),
            grpc_keepalive_timeout: Duration::from_millis(grpc_keepalive_timeout_ms),
            reflection_enabled,
            grpc_tls_cert,
            grpc_tls_key,
            grpc_rate_limit_rps,
        })
    }
}
