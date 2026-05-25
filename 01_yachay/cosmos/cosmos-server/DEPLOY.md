# Cosmobiología — guía de deploy

Server HTTP single-user, escrito en Rust + axum. Sirve cartas
astrológicas computadas con `cosmos-engine` (VSOP2013 en Rust
puro) y la página web HTML/JS del cliente. Diseñado para correr
**local** o detrás de un reverse proxy con TLS.

---

## 1. Build

### Binario del server

```bash
cargo build --release -p cosmos-server
# ./target/release/cosmos-server
```

### Cliente WASM (opcional pero recomendado)

Sin esto, el cliente cae al **SSR**: cada interacción pide al server
el SVG recompuesto (~12 KB por click). Con WASM, el cliente compone
localmente — primera carga ~150 KB, después scrubbing instantáneo
sin round-trip.

```bash
# Una sola vez:
cargo install wasm-pack

# Cada vez que cambie cosmos-render o cosmos-web:
cd 01_yachay/cosmos/cosmos-web
wasm-pack build --release --target web \
    --out-dir ../../../../apps/cosmos-server/static/wasm
```

`wasm-pack` produce `cosmobiologia_web.js` +
`cosmobiologia_web_bg.wasm` en
`01_yachay/cosmos/cosmos-server/static/wasm/`. El server los sirve
en `/static/wasm/*` y el `index.html` los importa con
`import init, { render_model_to_svg } from
'/static/wasm/cosmobiologia_web.js'`.

Si el directorio NO existe (build incompleto), el server devuelve
404 y el cliente cae al SSR automáticamente — sin error visible.

---

## 2. Levantar el server

### Local (single-user, sin reverse proxy)

```bash
./target/release/cosmos-server \
    --port 8787 \
    --bind 127.0.0.1 \
    --db ~/.local/share/cosmobiologia/charts.db
```

Abrí `http://127.0.0.1:8787/`. La DB es la misma que usa la app
desktop — cualquier carta creada en la app aparece en el browser
y viceversa.

### systemd (server público vía VPS)

```ini
# /etc/systemd/system/cosmobiologia.service
[Unit]
Description=Cosmobiología (server astrológico)
After=network.target

[Service]
Type=simple
User=cosmobio
Group=cosmobio
WorkingDirectory=/opt/cosmobiologia
ExecStart=/opt/cosmobiologia/cosmos-server \
    --port 8787 \
    --bind 127.0.0.1 \
    --db /var/lib/cosmobiologia/charts.db \
    --static-wasm /opt/cosmobiologia/static/wasm
Environment=RUST_LOG=cosmobiologia_server=info,tower_http=warn
Restart=on-failure
RestartSec=3
# Sandboxing básico
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
ReadWritePaths=/var/lib/cosmobiologia
NoNewPrivileges=true

[Install]
WantedBy=multi-user.target
```

```bash
sudo useradd -r -s /usr/sbin/nologin cosmobio
sudo mkdir -p /opt/cosmobiologia/static/wasm /var/lib/cosmobiologia
sudo cp target/release/cosmos-server /opt/cosmobiologia/
sudo cp -r 01_yachay/cosmos/cosmos-server/static/wasm/* \
    /opt/cosmobiologia/static/wasm/
sudo chown -R cosmobio:cosmobio /opt/cosmobiologia /var/lib/cosmobiologia
sudo systemctl daemon-reload
sudo systemctl enable --now cosmobiologia
sudo systemctl status cosmobiologia
```

---

## 3. Reverse proxy (HTTPS + DNS bonito)

Con dos subdominios apuntando al host:

| DNS | Función |
|-----|---------|
| `cosmobiologia.gioser.net` | página web (HTML + WASM) |
| `api.cosmobiologia.gioser.net` | endpoints `/api/*` (JSON / SVG) |

Hoy el server sirve los dos roles en el mismo puerto — el split por
subdominio lo hace el proxy, **sin cambiar nada del Rust**.

### Caddyfile (recomendado — TLS automático con Let's Encrypt)

```Caddyfile
cosmobiologia.gioser.net {
    encode gzip zstd
    # Página web + estáticos + WASM
    @api path /api/*
    handle @api {
        # Si el cliente pega un /api/ directo al subdominio principal,
        # lo dejamos pasar (más amigable que 404).
        reverse_proxy 127.0.0.1:8787
    }
    handle {
        reverse_proxy 127.0.0.1:8787
    }
}

api.cosmobiologia.gioser.net {
    encode gzip zstd
    # Solo los endpoints /api/*; rechaza el resto.
    @api path /api/*
    handle @api {
        reverse_proxy 127.0.0.1:8787
    }
    handle {
        respond "Use cosmobiologia.gioser.net para la página" 404
    }
}
```

### nginx (alternativa)

```nginx
# /etc/nginx/sites-available/cosmobiologia
server {
    server_name cosmobiologia.gioser.net;
    listen 443 ssl http2;
    # ssl_certificate / ssl_certificate_key — vía certbot
    location / {
        proxy_pass http://127.0.0.1:8787;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
    }
    gzip on;
    gzip_types application/javascript application/wasm image/svg+xml application/json text/css text/html;
}

server {
    server_name api.cosmobiologia.gioser.net;
    listen 443 ssl http2;
    location /api/ {
        proxy_pass http://127.0.0.1:8787;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
    }
    location / { return 404; }
    gzip on;
    gzip_types application/json image/svg+xml;
}
```

### DNS

A records (o AAAA si IPv6) hacia tu VPS:

```
cosmobiologia.gioser.net.       A  <ip-del-VPS>
api.cosmobiologia.gioser.net.   A  <ip-del-VPS>
```

---

## 4. CORS y separación cliente↔API

Hoy el server tiene `CorsLayer::permissive()` — cualquier origen
puede hacer fetch contra `/api/*`. Eso es OK para:

- **Single-user local**: nadie más alcanza al server.
- **Demo público single-tenant**: misma DB para todos los visitantes,
  sin datos sensibles. Los visitantes pueden leer y crear cartas
  públicamente (es la naturaleza del demo).

**No use CorsLayer::permissive en producción multi-usuario**. Para
eso hay que:
1. Agregar auth (sesiones / JWT / API key).
2. Reemplazar con `CorsLayer::new().allow_origin(["https://cosmobiologia.gioser.net".parse().unwrap()])`.
3. Volverte el `AllowCredentials::yes()` si vas a usar cookies.

---

## 5. Separación demo público ↔ desktop personal

El path por default de la DB (`~/.local/share/cosmobiologia/charts.db`)
es **compartido entre el server y la app desktop**. Eso es lo que
querés en tu máquina local — abrís el browser y ves las mismas
cartas que tenés en la app gpui.

**Pero NO querés que el server público en
`cosmobiologia.gioser.net` exponga TUS cartas privadas**. Para el
demo público:

```bash
# En tu VPS:
mkdir -p /var/lib/cosmobiologia
# Empezás con DB vacía (la app crea las tablas al primer arranque).
cosmos-server --db /var/lib/cosmobiologia/charts.db
```

Si querés precargar cartas demo (Einstein, una carta natal pública),
podés copiarlas desde tu DB local con la app, exportarlas como JSON
via `/api/charts/:id`, y postearlas al server público con POST
`/api/charts`. O simplemente abrir el browser, ir a "Nuevo
contacto" → "Nueva carta…" y cargarlas a mano.

---

## 6. Backup

La DB SQLite es **un solo archivo**. Backup = `cp` (mientras el
server está parado, o usá `sqlite3 charts.db ".backup
charts.bak"` con el server corriendo).

```bash
# Snapshot diario sin parar el server
sqlite3 /var/lib/cosmobiologia/charts.db ".backup /var/backups/cosmobiologia-$(date +%F).db"
```

---

## 7. Smoke test post-deploy

```bash
# Desde tu máquina:
curl https://cosmobiologia.gioser.net/api/health
# → {"status":"ok","service":"cosmos-server"}

curl https://cosmobiologia.gioser.net/api/sky | jq .title
# → "Cielo 2026-05-19 00:55 UTC"

# Abrí la página:
open https://cosmobiologia.gioser.net/
# (deberías ver la rueda del cielo + sidebar con "Cielo ahora")
```

Si el cliente WASM cargó, en la barra inferior verás "WASM".
Si cayó al SSR, verás "SSR". Ambos modos son funcionales.

---

## 8. Endpoints públicos (referencia)

| Método | Path | Función |
|--------|------|---------|
| GET | `/api/health` | healthcheck |
| GET | `/api/tree` | árbol completo (groups/contacts/charts) |
| GET | `/api/sky` | RenderModel "Cielo ahora" |
| GET | `/api/sky.svg` | SVG agnóstico del cielo (server-side) |
| GET | `/api/charts/:id` | Chart JSON |
| GET | `/api/charts/:id/render?...` | RenderModel con overlays |
| GET | `/api/charts/:id/svg?...` | SVG vía engine (svg_export) |
| GET | `/api/charts/:id/wheel.svg?...` | SVG vía render agnóstico |
| POST | `/api/charts` | crear carta |
| PATCH | `/api/charts/:id` | editar label/birth/config |
| DELETE | `/api/charts/:id` | borrar |
| POST | `/api/groups` | crear grupo |
| PATCH | `/api/groups/:id` | renombrar |
| DELETE | `/api/groups/:id` | borrar |
| POST | `/api/contacts` | crear contacto |
| PATCH | `/api/contacts/:id` | renombrar |
| DELETE | `/api/contacts/:id` | borrar |

Query params del render (`?...`):

- `offset_min=<i64>` — time scrubbing (minutos desde el natal).
- `transit=1` — activa overlay de tránsito al `now` del server.
- `prog_age=<f64>` — progresión secundaria a edad N.
- `sa_age=<f64>` — solar arc a edad N.
- `pd_age=<f64>` — primary directions GR (Naibod).
