use clap::Parser;
use gungnir::cli::Cli;

fn main() -> eyre::Result<()> {
    Cli::parse().run()
}
