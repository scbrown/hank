//! The `hank` binary entrypoint. See the `hank` library crate and
//! `docs/hank-spec.md` for the design.

use clap::Parser;

fn main() -> anyhow::Result<()> {
    hank::cli::init_tracing();
    let cli = hank::cli::Cli::parse();
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(cli.run())
}
