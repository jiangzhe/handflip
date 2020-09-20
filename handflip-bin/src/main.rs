use structopt::StructOpt;
use handflip_core::http::HttpProxy;
use anyhow::Result;

fn main() -> Result<()> {
    env_logger::init();
    let opts = Opts::from_args();
    let addr = format!("{}:{}", opts.host, opts.port);
    let http_proxy = if let Some(socks5) = opts.socks5 {
        HttpProxy::via_socks5(socks5)
    } else {
        HttpProxy::direct()
    };
    log::debug!("listening at {}", addr);
    smol::block_on(http_proxy.bind(addr))?;
    Ok(())
}

#[derive(Debug, StructOpt)]
pub struct Opts {
    #[structopt(short, long, default_value = "127.0.0.1")]
    pub host: String,
    #[structopt(short, long, default_value = "1081")]
    pub port: u16,
    #[structopt(short, long, help = "specify socks5 proxy address, e.g. 127.0.0.1:1080")]
    pub socks5: Option<String>,
}
