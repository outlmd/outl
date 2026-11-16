use std::process::ExitCode;
use std::time::Duration;

use discussions_discord::ctx::{Ctx, Env};
use discussions_discord::ureq_http::UreqHttp;

fn main() -> ExitCode {
    let env: Env = std::env::vars().collect();
    let http = UreqHttp::new();
    let ctx = Ctx {
        env: &env,
        http: &http,
        sleep: &|wait: Duration| std::thread::sleep(wait),
        log: &|line: &str| println!("{line}"),
    };
    // Exit codes are 0 or 1.
    ExitCode::from(discussions_discord::execute(&ctx) as u8)
}
