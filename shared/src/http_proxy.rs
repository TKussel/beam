use std::{time::Duration, collections::HashSet};

use http::Uri;
use hyper::{Client, client::{HttpConnector, connect::Connect, conn}, service::Service};
use hyper_proxy::{Intercept, Proxy, ProxyConnector, Custom};
use hyper_tls::{HttpsConnector, native_tls::{TlsConnector, Certificate}};
use mz_http_proxy::hyper::connector;
use once_cell::sync::OnceCell;
use openssl::x509::X509;
use tracing::{debug, info};

use crate::{config, errors::SamplyBeamError, BeamId};

pub fn build_hyper_client(ca_certificates: &Vec<X509>) -> Result<Client<ProxyConnector<HttpsConnector<HttpConnector>>>, std::io::Error> {
    let mut http = HttpConnector::new();
    http.set_connect_timeout(Some(Duration::from_secs(1)));
    http.enforce_http(false);
    let https = HttpsConnector::new_with_connector(http);
    let proxy_connector = connector()
        .map_err(|e| panic!("Unable to build HTTP client: {}", e)).unwrap();
    let mut proxy_connector = proxy_connector.with_connector(https);
    if ! ca_certificates.is_empty() {
        let mut tls = TlsConnector::builder();
        for cert in ca_certificates {
            const ERR: &str = "Internal Error: Unable to convert Certificate.";
            let cert = Certificate::from_pem(&cert.to_pem().expect(ERR)).expect(ERR);
            tls.add_root_certificate(cert);
        }
        let tls = tls
            .build()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("Unable to build TLS Connector with custom CA certificates: {}", e)))?;
        proxy_connector.set_tls(Some(tls));
    }

    let proxies = proxy_connector.proxies().iter()
        .map(|p| p.uri().to_string())
        .collect::<HashSet<_>>();

    if proxies.len() == 0 && ca_certificates.len() > 0 {
        return Err(std::io::Error::new(std::io::ErrorKind::Other, "Certificates for TLS termination were provided but no proxy to use. Please supply correct configuration."));
    }

    let proxies = match proxies.len() {
        0 => "no proxy".to_string(),
        1 => format!("proxy {}", proxies.iter().next().unwrap()),
        num => format!("{num} proxies {:?}", proxies)
    };
    let certs = match ca_certificates.len() {
        0 => "no trusted certificate".to_string(),
        1 => "a trusted certificate".to_string(),
        num => format!("{num} trusted certificates")
    };
    info!("Using {proxies} and {certs} for TLS termination.");
    
    Ok(Client::builder().build(proxy_connector))
}

#[cfg(test)]
mod test {

    use std::path::{Path, PathBuf};

    use hyper::{Client, client::{HttpConnector, connect::Connect}, Uri, Request, body};
    use hyper_proxy::ProxyConnector;
    use hyper_tls::HttpsConnector;
    use openssl::x509::X509;

    use super::build_hyper_client;

    const HTTP: &str = "http://ip-api.com/json";
    const HTTPS: &str = "https://ifconfig.me/";

    fn get_certs() -> Vec<X509> {
        if let Ok(dir) = std::env::var("TLS_CA_CERTIFICATES_DIR") {
            let dir = PathBuf::from(dir);
            crate::crypto::load_certificates_from_dir(Some(dir)).unwrap()
        } else {
            Vec::new()
        }
    }

    #[tokio::test]
    async fn https() {
        let client = build_hyper_client(&get_certs()).unwrap();
        run(HTTPS.parse().unwrap(), client).await;
    }

    #[tokio::test]
    async fn http() {
        let client = build_hyper_client(&get_certs()).unwrap();
        run(HTTP.parse().unwrap(), client).await;
    }

    async fn run(url: Uri, client: Client<impl Connect + Clone + Send + Sync + 'static>) {
        let req = Request::builder()
            .uri(url)
            .body(body::Body::empty())
            .unwrap();

        let mut resp = client.request(req).await.unwrap();

        let resp_string = body::to_bytes(resp.body_mut()).await.unwrap();

        println!("=> {}\n", std::str::from_utf8(&resp_string).unwrap());
    }
}