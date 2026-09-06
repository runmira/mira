use std::collections::BTreeMap;
use mira_config::{McpServerConfig, McpStdioConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = McpServerConfig::Stdio(McpStdioConfig {
        command: "npx".into(),
        args: vec!["-y".into(), "@modelcontextprotocol/server-everything".into()],
        env: BTreeMap::new(),
        cwd: None,
    });
    let conn = mira_tools::connect_mcp("everything", &cfg).await?;
    println!("connected: {} tools", conn.tools.len());
    for t in &conn.tools {
        let s = t.spec();
        println!("  - {} ({})", s.name, s.description.chars().take(60).collect::<String>());
    }
    Ok(())
}
