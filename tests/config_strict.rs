//! STRICT_CONFIG rejects unknown QUOTE_LEDGER_* / APPEND_* / GRPC_* keys.

use std::sync::Mutex;

use quote_ledger::ServiceConfig;

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn strict_config_rejects_unknown_prefixed_env() {
    let _g = ENV_LOCK.lock().expect("env lock");

    std::env::set_var("STRICT_CONFIG", "true");
    std::env::set_var("GRPC_LISTEN_ADDR", "127.0.0.1:59999");
    std::env::set_var("QUOTE_LEDGER_TYPOS_SHOULD_FAIL", "1");

    let err = ServiceConfig::from_env_and_args().expect_err("strict should fail");
    assert!(
        err.contains("unknown environment variable")
            || err.contains("QUOTE_LEDGER_TYPOS_SHOULD_FAIL"),
        "unexpected error: {err}"
    );

    std::env::remove_var("STRICT_CONFIG");
    std::env::remove_var("QUOTE_LEDGER_TYPOS_SHOULD_FAIL");
    std::env::remove_var("GRPC_LISTEN_ADDR");
}
