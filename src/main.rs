use clap::Parser;

#[derive(Parser)]
#[command(
    version,
    about = "Benchmark coding agents across programming languages and reasoning effort"
)]
struct Cli {}

fn main() -> eyre::Result<()> {
    let _cli = Cli::parse();
    Ok(())
}
