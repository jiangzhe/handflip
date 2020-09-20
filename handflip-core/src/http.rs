use async_net::{AsyncToSocketAddrs, TcpListener, TcpStream};
use crate::error::{Result, Error};
use async_h1::{server, client};
use futures::{io, future, StreamExt, AsyncWriteExt};
use http_types::{Request, Response, StatusCode, Method};
use crate::socks5;

#[derive(Debug)]
pub struct HttpProxy {
    transport: Transport,
}

impl HttpProxy {

    pub fn direct() -> Self {
        Self{
            transport: Transport::Direct,
        }
    }

    pub fn via_socks5(socks5: String) -> Self {
        Self{
            transport: Transport::Socks5(socks5),
        }
    }

    /// bind to given address
    pub async fn bind(&self, addr: impl AsyncToSocketAddrs) -> Result<()> {
        let listener = TcpListener::bind(addr).await?;
        listener.incoming().for_each_concurrent(None, |conn| async {
            match conn {
                Ok(conn) => {
                    if let Err(e) = self.handle(conn).await {
                        log::debug!("error handling connection {}", e);
                    }
                },
                Err(e) => {
                    log::debug!("error accepting connection {}", e);
                }
            }
        }).await;
        Ok(())
    }

    async fn handle(&self, conn: TcpStream) -> Result<()> {
        if let Some(req) = server::decode(conn.clone()).await? {
            self.handle_request(conn, req).await?;
        }
        Ok(())
    }

    async fn handle_request(&self, conn: TcpStream, req: Request) -> Result<()> {
        log::debug!("req from {:?}={:#?}", conn.peer_addr(), req);
        match req.method() {
            Method::Connect => self.handle_connect_request(conn, req).await?,
            _ => self.handle_other_request(conn, req).await?,
        }
        Ok(())
    }

    async fn handle_connect_request(&self, mut conn: TcpStream, req: Request) -> Result<()> {
        let (host, port) = host_port_from_req(&req)?;
        let upstream_addr = format!("{}:{}", host, port);
        log::debug!("try to connect to {}", upstream_addr);
        let upstream = match self.transport.connect(&upstream_addr).await {
            Ok(stream) => {
                stream
            }
            Err(e) => {
                log::debug!("Error connecting to upstream {}", e);
                let resp = Response::new(StatusCode::ServiceUnavailable);
                let encoder = server::Encoder::new(resp, req.method());
                io::copy(encoder, &mut conn).await?;
                return Ok(());
            }
        };
        log::debug!("connected to {}", upstream_addr);
        // send back response to notify client the proxy initialization succeeds
        // follow rfc7231#section-4.3.6: do not send Content-Length header
        conn.write_all(b"HTTP/1.1 200 Connection established\r\n\r\n").await?;
        log::debug!("send CONNECT response 200 to client");
    
        // forward two streams
        keep_alive_proxy(conn, upstream).await
    }

    async fn handle_other_request(&self, mut conn: TcpStream, mut req: Request) -> Result<()> {
        let (host, port) = host_port_from_req(&req)?;
        let keep_alive = if let Some(pc) = req.header("Proxy-Connection") {
            pc == "Keep-Alive"
        } else {
            false
        };
        let upstream_addr = format!("{}:{}", host, port);
        log::debug!("try to connect to {}", upstream_addr);
        let mut upstream = match self.transport.connect(&upstream_addr).await {
            Ok(stream) => {
                stream
            }
            Err(e) => {
                log::debug!("Error connecting to upstream {}", e);
                let resp = Response::new(StatusCode::ServiceUnavailable);
                let encoder = server::Encoder::new(resp, req.method());
                io::copy(encoder, &mut conn).await?;
                return Ok(());
            }
        };
        log::debug!("connected to {}", upstream_addr);
        req.remove_header("Proxy-Connection");
        if keep_alive {
            log::debug!("keep-alive enabled on upstream connection");
            // for http 1.0, add keep-alive header
            req.insert_header("Connection", "Keep-Alive");
            // send the initial request to upstream
            // let resp = client::connect(conn.clone(), req).await?;
            let encoder = client::Encoder::encode(req).await?;
            io::copy(encoder, &mut upstream).await?;
            keep_alive_proxy(conn, upstream).await?;
            return Ok(());
        }
        // not keep-alive, send and close connection
        req.insert_header("Connection", "close");
        let req_method = req.method();
        let mut resp = client::connect(upstream, req).await?;
        log::debug!("original response={:#?}", resp);
        resp.insert_header("Connection", "close");
        let encoder = server::Encoder::new(resp, req_method);
        io::copy(encoder, &mut conn).await?;
        Ok(())
    }
}

#[derive(Debug)]
pub enum Transport {
    Direct,
    Socks5(String),
}

impl Transport {
    pub async fn connect(&self, target: impl AsyncToSocketAddrs) -> Result<TcpStream> {
        let conn = match self {
            Transport::Direct => {
                TcpStream::connect(target).await?
            }
            Transport::Socks5(proxy) => {
                socks5::client::proxy(proxy, target).await?
            }
        };
        Ok(conn)
    }
}

async fn keep_alive_proxy(conn: TcpStream, upstream: TcpStream) -> Result<()> {
    let mut conn_writer = conn.clone();
    let mut upstream_writer = upstream.clone();
    let proxy_result = future::select(
        io::copy(conn, &mut upstream_writer),
        io::copy(upstream, &mut conn_writer),
    ).await;

    match proxy_result {
        future::Either::Left(left) => {
            let (req, _) = left;
            let req = req?;
            log::debug!("sended request bytes {}, probably client closed the connection", req);
        }
        future::Either::Right(right) => {
            let (resp, _) = right;
            let resp = resp?;
            log::debug!("received response bytes {}, probably upstream closed the connection", resp);
        }
    }
    Ok(())
}

/// fetch target host and port from the request
/// 
/// according to rfc7230#section-5.3
#[inline]
fn host_port_from_req(req: &Request) -> Result<(&str, u16)> {
    let scheme = req.url().scheme();
    let host_str = req.url().host_str()
        .ok_or_else(|| Error::Http("missing host in url".to_owned()))?;
    let host_splits: Vec<_> = host_str.split(':').collect();
    if host_splits.len() == 2 {
        let host = host_splits[0];
        let port: u16 = host_splits[1].parse()?;
        Ok((host, port))
    } else {
        let host = host_splits[0];
        let port = if let Some(port) = req.url().port() {
            port
        } else if scheme == "https" {
            443
        } else {
            80
        };
        Ok((host, port))
    }
}
