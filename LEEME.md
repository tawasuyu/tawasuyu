# tawasuyu

Monorepo organizado por el ciclo de la información en 4 cuadrantes.

```
tawasuyu/
├── 00_unanchay/   PERCIBIR  — pluma, khipu, rimay, chaka, pineal, puriy
├── 01_yachay/     CONOCER   — cosmos, dominium, nakui
├── 02_ruway/      HACER     — mirada, shuma, nahual, chasqui, takiy
├── 03_ukupacha/   RAÍZ      — arje, wawa, agora, minga
├── shared/                  — sandokan, auth, card, ssh, format
└── web/                     — landing (interno, no producto)
```

## Principios

1. **Filesystem = arquitectura.** Cada cuadrante es una fase del ciclo de información.
2. **Un dominio = un crate raíz con subcrates plugin.** Sin proliferación.
3. **UIs son frontends intercambiables** sobre `*-core` agnósticos.
4. **Nombres con carga semántica fuerte se respetan** sea cual sea su idioma.

## Cuadrantes

### 00_unanchay — Percibir
Ingesta de información: documentos vivos (pluma), notas (khipu), lenguaje (rimay), legacy bridge (chaka), viz (pineal), navegador web (puriy).

### 01_yachay — Conocer
Modelos del mundo: astrología (cosmos), simulación (dominium), ERP (nakui).

### 02_ruway — Hacer
Interfaces y ejecución: shell gráfico (mirada), runtime de espacios (shuma),
renderer GPU (nahual), message broker (chasqui), composición musical (takiy),
motor gráfico soberano (llimphi).

### 03_ukupacha — Raíz
Base inamovible: init (arje), kernel SASOS (wawa), identidad (agora), P2P VFS (minga).
