use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use promissory_core::network::SoNetwork;
use promissory_keeper::{
    api,
    backend::{Beacon, Chain, DrandBeacon, LwdChain},
    db::Db,
    keeper::{Config, Keeper},
    limits::{Limits, RateKey},
    webhook::Notifier,
};

#[derive(Parser)]
#[command(name = "promissory-keeper", about = "Promissory release service")]
struct Cli {
    #[arg(long, default_value = "127.0.0.1:8737")]
    listen: SocketAddr,
    #[arg(long, default_value = "keeper.db")]
    db: PathBuf,
    /// Chain to use: `main`, or `regtest` (the local chain from regtest/regtest.sh)
    #[arg(long, default_value = "main")]
    network: NetworkArg,
    /// lightwalletd endpoint [default: zec.rocks on main, 127.0.0.1:9067 on regtest]
    #[arg(long)]
    lwd: Option<String>,
    /// NU7 activation height of the regtest chain [default: 5000, as in regtest/zebrad.toml.tmpl]
    #[arg(long)]
    nu7_height: Option<u32>,
    /// Accept webhook URLs on upload and POST state changes to them
    #[arg(long)]
    allow_webhooks: bool,
    #[arg(long, default_value_t = 3)]
    tick_secs: u64,

    /// Refuse uploads locked to a round more than this many days ahead
    #[arg(long, default_value_t = 400)]
    max_horizon_days: u64,
    /// Per-client upload buckets keyed by `ip` (salted hash, memory only, rotated daily; never
    /// logged) or `none`. Use `none` behind Tor or a proxy, where every client shares one address.
    #[arg(long, value_enum, default_value = "ip")]
    rate_limit_key: RateKey,
    /// Per-client uploads per minute (sustained)
    #[arg(long, default_value_t = 60)]
    client_rate: u32,
    /// Per-client burst of uploads; 0 disables per-client limits
    #[arg(long, default_value_t = 300)]
    client_burst: u32,
    /// Uploads per minute across all clients (sustained)
    #[arg(long, default_value_t = 1200)]
    global_rate: u32,
    /// Burst of uploads across all clients; 0 disables the global limit
    #[arg(long, default_value_t = 3000)]
    global_burst: u32,
    /// Cap on ciphertext held, in MiB (uploads beyond it get 507); 0 disables
    #[arg(long, default_value_t = 1024)]
    max_stored_mib: u64,
    /// Cap on locked blobs (uploads beyond it get 507); 0 disables
    #[arg(long, default_value_t = 100_000)]
    max_locked_blobs: u64,

    /// Reach lightwalletd, drand and webhook receivers through Tor. Each broadcast uses a fresh
    /// circuit. The keeper refuses to start if Tor can't bootstrap; it never falls back to
    /// clearnet.
    #[arg(long)]
    tor: bool,
    /// Tor (arti) state and cache directory [default: `tor` next to --db]
    #[arg(long)]
    tor_dir: Option<PathBuf>,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum NetworkArg {
    Main,
    Regtest,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "promissory_keeper=info".into()),
        )
        .init();
    let cli = Cli::parse();
    let (params, default_lwd) = match cli.network {
        NetworkArg::Main => (SoNetwork::Main, "https://zec.rocks:443"),
        NetworkArg::Regtest => (
            cli.nu7_height.map_or_else(SoNetwork::regtest, SoNetwork::regtest_with_nu7),
            "http://127.0.0.1:9067",
        ),
    };
    let lwd = cli.lwd.unwrap_or_else(|| default_lwd.to_string());
    let db = Db::open(&cli.db).with_context(|| format!("opening {}", cli.db.display()))?;
    let (beacon, chain, notifier) = if cli.tor {
        let dir = cli.tor_dir.clone().unwrap_or_else(|| {
            cli.db.parent().unwrap_or(std::path::Path::new("")).join("tor")
        });
        tor_backends(&dir, &lwd, cli.allow_webhooks).await?
    } else {
        let beacon: Arc<dyn Beacon> = Arc::new(DrandBeacon::quicknet()?);
        let chain: Arc<dyn Chain> = Arc::new(LwdChain::new(&lwd));
        (beacon, chain, cli.allow_webhooks.then(Notifier::new).transpose()?)
    };
    let cfg = Config {
        tick: Duration::from_secs(cli.tick_secs.max(1)),
        ..Config::default()
    };
    let keeper = Arc::new(Keeper::new(db, beacon, chain, params, cfg, notifier));
    let limits = Limits {
        max_horizon_days: cli.max_horizon_days,
        rate_key: cli.rate_limit_key,
        client_per_min: cli.client_rate,
        client_burst: cli.client_burst,
        global_per_min: cli.global_rate,
        global_burst: cli.global_burst,
        max_stored_bytes: cli.max_stored_mib.saturating_mul(1 << 20),
        max_locked_blobs: cli.max_locked_blobs,
    };

    tokio::spawn(keeper.clone().run());
    let listener = tokio::net::TcpListener::bind(cli.listen)
        .await
        .with_context(|| format!("binding {}", cli.listen))?;
    tracing::info!(
        listen = %cli.listen, %lwd, webhooks = cli.allow_webhooks, tor = cli.tor,
        rate_limit_key = cli.rate_limit_key.as_str(), "keeper up"
    );
    let app = api::router_with(keeper, limits);
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

type Backends = (Arc<dyn Beacon>, Arc<dyn Chain>, Option<Notifier>);

#[cfg(feature = "tor")]
async fn tor_backends(dir: &std::path::Path, lwd: &str, webhooks: bool) -> Result<Backends> {
    use promissory_keeper::backend::TorDrandBeacon;
    tracing::info!(dir = %dir.display(), "bootstrapping Tor");
    let tor = promissory_core::tor::bootstrap(dir)
        .await
        .context("refusing to start without Tor (drop --tor to use clearnet)")?;
    tracing::info!(
        secs = promissory_core::tor::bootstrap_time().map(|d| d.as_secs_f32()),
        "Tor bootstrapped"
    );
    let beacon: Arc<dyn Beacon> = Arc::new(TorDrandBeacon::new(&tor));
    let chain: Arc<dyn Chain> = Arc::new(LwdChain::tor(lwd, &tor));
    Ok((beacon, chain, webhooks.then(|| Notifier::tor(&tor))))
}

#[cfg(not(feature = "tor"))]
async fn tor_backends(_: &std::path::Path, _: &str, _: bool) -> Result<Backends> {
    anyhow::bail!("this promissory-keeper was built without Tor; rebuild with `--features tor`")
}
