use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "kraalzibar-server", version)]
pub struct Cli {
    #[arg(short, long, global = true)]
    pub config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Serve,
    Migrate,
    ProvisionTenant {
        #[arg(long)]
        name: String,
    },
    CreateApiKey {
        #[arg(long)]
        tenant_name: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn cli_parses_serve_subcommand() {
        let cli = Cli::parse_from(["kraalzibar-server", "serve"]);
        assert!(matches!(cli.command, Some(Command::Serve)));
    }

    #[test]
    fn cli_parses_migrate_subcommand() {
        let cli = Cli::parse_from(["kraalzibar-server", "migrate"]);
        assert!(matches!(cli.command, Some(Command::Migrate)));
    }

    #[test]
    fn cli_parses_config_flag() {
        let cli = Cli::parse_from(["kraalzibar-server", "--config", "/etc/kraalzibar.toml"]);
        assert_eq!(cli.config, Some(PathBuf::from("/etc/kraalzibar.toml")));
        assert!(cli.command.is_none());
    }

    #[test]
    fn cli_defaults_to_no_subcommand() {
        let cli = Cli::parse_from(["kraalzibar-server"]);
        assert!(cli.command.is_none());
        assert!(cli.config.is_none());
    }

    #[test]
    fn cli_config_flag_works_with_migrate() {
        let cli = Cli::parse_from([
            "kraalzibar-server",
            "--config",
            "/etc/kraalzibar.toml",
            "migrate",
        ]);
        assert_eq!(cli.config, Some(PathBuf::from("/etc/kraalzibar.toml")));
        assert!(matches!(cli.command, Some(Command::Migrate)));
    }

    #[test]
    fn cli_parses_provision_tenant() {
        let cli = Cli::parse_from(["kraalzibar-server", "provision-tenant", "--name", "acme"]);
        assert!(matches!(
            cli.command,
            Some(Command::ProvisionTenant { name }) if name == "acme"
        ));
    }

    #[test]
    fn cli_parses_create_api_key() {
        let cli = Cli::parse_from([
            "kraalzibar-server",
            "create-api-key",
            "--tenant-name",
            "acme",
        ]);
        assert!(matches!(
            cli.command,
            Some(Command::CreateApiKey { tenant_name }) if tenant_name == "acme"
        ));
    }

    #[test]
    fn cli_config_flag_works_after_subcommand() {
        let cli = Cli::parse_from([
            "kraalzibar-server",
            "serve",
            "--config",
            "/etc/kraalzibar.toml",
        ]);
        assert_eq!(cli.config, Some(PathBuf::from("/etc/kraalzibar.toml")));
        assert!(matches!(cli.command, Some(Command::Serve)));
    }

    #[test]
    fn cli_version_flag() {
        let result = Cli::try_parse_from(["kraalzibar-server", "--version"]);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::DisplayVersion);
    }
}
