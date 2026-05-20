//! Renderizado de un [`VHost`] a un bloque `server` de nginx.

use matilda_core::{Upstream, VHost};

/// URL de `proxy_pass` para un upstream. Un contenedor se referencia por
/// su nombre, que la red de Docker resuelve a su IP interna.
fn proxy_target(upstream: &Upstream) -> String {
    match upstream {
        Upstream::Address(addr) => format!("http://{addr}"),
        Upstream::Container { name, port } => format!("http://{name}:{port}"),
    }
}

/// Renderiza el `server` de nginx de un vhost. Con TLS emite dos
/// bloques: el `:443 ssl` y un `:80` que redirige a HTTPS.
pub fn nginx_server_block(v: &VHost) -> String {
    let names: Vec<&str> = std::iter::once(v.domain.as_str())
        .chain(v.aliases.iter().map(|s| s.as_str()))
        .collect();
    let server_name = names.join(" ");
    let target = proxy_target(&v.upstream);

    let mut out = String::new();
    if v.tls {
        // Redirección :80 → :443.
        out.push_str("server {\n");
        out.push_str("    listen 80;\n");
        out.push_str(&format!("    server_name {server_name};\n"));
        out.push_str("    return 301 https://$host$request_uri;\n");
        out.push_str("}\n\n");

        out.push_str("server {\n");
        out.push_str("    listen 443 ssl;\n");
        out.push_str(&format!("    server_name {server_name};\n"));
        out.push_str(&format!(
            "    ssl_certificate /etc/letsencrypt/live/{}/fullchain.pem;\n",
            v.domain
        ));
        out.push_str(&format!(
            "    ssl_certificate_key /etc/letsencrypt/live/{}/privkey.pem;\n",
            v.domain
        ));
    } else {
        out.push_str("server {\n");
        out.push_str("    listen 80;\n");
        out.push_str(&format!("    server_name {server_name};\n"));
    }

    out.push_str("    location / {\n");
    out.push_str(&format!("        proxy_pass {target};\n"));
    out.push_str("        proxy_set_header Host $host;\n");
    out.push_str("        proxy_set_header X-Real-IP $remote_addr;\n");
    out.push_str("    }\n");
    out.push_str("}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_vhost_listens_on_80() {
        let block = nginx_server_block(&VHost::to_container("app.com", "web", 8080));
        assert!(block.contains("listen 80;"));
        assert!(!block.contains("listen 443"));
        assert!(block.contains("server_name app.com;"));
        assert!(block.contains("proxy_pass http://web:8080;"));
    }

    #[test]
    fn tls_vhost_adds_443_and_redirect() {
        let block = nginx_server_block(&VHost::to_address("secure.com", "10.0.0.5:80").with_tls());
        assert!(block.contains("listen 443 ssl;"));
        assert!(block.contains("return 301 https://$host$request_uri;"));
        assert!(block.contains("/etc/letsencrypt/live/secure.com/fullchain.pem"));
        assert!(block.contains("proxy_pass http://10.0.0.5:80;"));
    }

    #[test]
    fn aliases_join_the_server_name() {
        let v = VHost::to_address("main.com", "1.2.3.4:80")
            .with_alias("www.main.com")
            .with_alias("alt.com");
        let block = nginx_server_block(&v);
        assert!(block.contains("server_name main.com www.main.com alt.com;"));
    }

    #[test]
    fn render_is_deterministic() {
        let v = VHost::to_container("x.com", "c", 80).with_tls();
        assert_eq!(nginx_server_block(&v), nginx_server_block(&v));
    }
}
