use async_net::{AsyncToSocketAddrs, TcpStream, resolve};
use crate::error::{Result, Error};
use futures::{AsyncWriteExt, AsyncReadExt};
use std::net::{SocketAddr, SocketAddrV4};

pub async fn proxy(proxy_addr: impl AsyncToSocketAddrs, target_addr: impl AsyncToSocketAddrs) -> Result<TcpStream> {
    let target_addrs = resolve(target_addr).await?;
    let target_addr = target_addrs.into_iter().filter_map(|addr| match addr {
        SocketAddr::V4(v4) => Some(v4),
        SocketAddr::V6(_) => None,
    }).next()
        .ok_or_else(|| Error::BadRequest("unknown host".to_owned()))?;
    log::debug!("resolve host to {:?}", target_addr);
    let mut conn = TcpStream::connect(proxy_addr).await?;
    log::debug!("connected to proxy addr");
    handshake(&mut conn).await?;
    log::debug!("handshake succeeded");
    send(&mut conn, target_addr).await?;
    log::debug!("send succeeded");
    receive(&mut conn).await?;
    log::debug!("receive succeeded");
    Ok(conn)
}

async fn handshake(conn: &mut TcpStream) -> Result<()> {
    // version=5, methods=1, method=no auth
    let req = [5u8, 1, 0];
    conn.write_all(&req[..]).await?;
    log::debug!("send negotiation request to server {:?}", req);
    let mut buf = [0u8;2];
    conn.read_exact(&mut buf).await?;
    log::debug!("receive negotiation response from server {:?}", buf);
    Ok(())
}

async fn send(conn: &mut TcpStream, addr: SocketAddrV4) -> Result<()> {
    // version=5, cmd=connect, reserve=0, addr_type=ipv4
    let mut req = vec![5u8, 1, 0, 1];
    req.extend_from_slice(&addr.ip().octets());
    req.extend_from_slice(&addr.port().to_be_bytes());
    conn.write_all(&req[..]).await?;
    Ok(())
}

async fn receive(conn: &mut TcpStream) -> Result<()> {
    let mut buf = [0u8; 1024];
    conn.read_exact(&mut buf[..2]).await?;
    debug_assert_eq!(5, buf[0]);
    match buf[1] {
        0x00 => (),
        0x01 => return Err(Error::Server("general socks server failure".to_owned())),
        0x02 => return Err(Error::Server("connection not allowed by ruleset".to_owned())),
        0x03 => return Err(Error::Server("network unreachable".to_owned())),
        0x04 => return Err(Error::Server("host unreachable".to_owned())),
        0x05 => return Err(Error::Server("connection refused".to_owned())),
        0x06 => return Err(Error::Server("ttl expired".to_owned())),
        0x07 => return Err(Error::Server("command not supported".to_owned())),
        0x08 => return Err(Error::Server("address type not supported".to_owned())),
        _ => return Err(Error::Server(format!("unknown socks server error type {}", buf[1]))),
    }
    conn.read_exact(&mut buf[..2]).await?;
    debug_assert_eq!(0, buf[0]);
    let addr = match buf[1] {
        0x01 => {
            conn.read_exact(&mut buf[..4]).await?;
            Addr::IPv4([buf[0], buf[1], buf[2], buf[3]])
        }
        0x03 => {
            conn.read_exact(&mut buf[..1]).await?;
            let domain_len = buf[0] as usize;
            conn.read_exact(&mut buf[..domain_len]).await?;
            let domain = String::from_utf8(Vec::from(&buf[..domain_len])).unwrap();
            Addr::DomainName(domain)
        }
        0x04 => {
            conn.read_exact(&mut buf[..16]).await?;
            Addr::IPv6([
                buf[0], buf[1], buf[2], buf[3],
                buf[4], buf[5], buf[6], buf[7],
                buf[8], buf[9], buf[10], buf[11],
                buf[12], buf[13], buf[14], buf[15],
            ])
        }
        _ => return Err(Error::Server("unsupported address type".to_owned())),
    };
    let port = {
        conn.read_exact(&mut buf[..2]).await?;
        ((buf[0] as u16) << 8) + buf[1] as u16
    };
    log::debug!("connect succeeded with addr={:?}, port={}", addr, port);
    Ok(())
}

#[derive(Debug, Clone)]
pub enum Addr {
    IPv4([u8;4]),
    DomainName(String),
    IPv6([u8;16]),
}