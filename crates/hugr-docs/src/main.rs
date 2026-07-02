use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use hugr_docs::{DocsConfig, answer_question};

#[derive(Parser)]
#[command(
    name = "hugr-docs",
    version,
    about = "Answer questions from a read-only docs folder as JSON"
)]
struct Cli {
    /// Folder containing the documentation archive.
    docs_path: PathBuf,

    /// Question to answer from the documentation.
    question: Vec<String>,

    /// Override the model id. Defaults to HUGR_DOCS_MODEL, then google/gemma-4-31B-it:cerebras.
    #[arg(short = 'm', long = "model")]
    model: Option<String>,

    /// Pretty-print JSON output.
    #[arg(long = "pretty")]
    pretty: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let question = cli.question.join(" ");
    anyhow::ensure!(!question.trim().is_empty(), "question cannot be empty");

    let config = DocsConfig::from_env(cli.docs_path.clone(), cli.model.clone())?;
    let answer = answer_question(config, &question).await?;
    if cli.pretty {
        println!("{}", serde_json::to_string_pretty(&answer)?);
    } else {
        println!("{}", serde_json::to_string(&answer)?);
    }
    Ok(())
}
