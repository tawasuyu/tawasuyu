<!-- Quechua (Cusco/Collao). Revisión por hablante nativo bienvenida. -->

# 03 ukupacha · saphi

`ukupacha` (runa-simi: *ukupacha — ukhu pacha, saphi, allpa ukhupi*). Kaymi **mana rikukuq cimiento** *cuadrante*: kernel, bootloader, filesystem, ñankuna ruraqkuna ukhupi, sistema sayachiq runakuna ayllu. Mana pi runapas chiqaqta rikun, paymi kamachin: sistema haqariy atin manapas.

Kay cuadrantepa kamachiyninmi **manaraq feature, ñawpaqman invariante**: `ukupacha`-pi rumi-tikrayqa llapallan árbolpa migración kuska valuq; chayraykun sapanka decisión `chunka watapis, kayqa kaqlla cheqaq kanmancha?` hina yuyaspa. Kaypi tikraqa allillamanta, sumaq yuyayllapaq.

## Aplicaciones

- **[agora](agora/README.md)** — rikuq llaqta. Foro, rimaykuy, deliberar pisi identidadwan.
- **[arje](arje/README.md)** — bootloader + sistemata kawsachiq. `arje-seeds` (muhukuna), `arje-packager` (huñuy), `arje-installer` (churay), `arje-absorb` (kawsaq sistemata hap'iy).
- **[minga](minga/README.md)** — nodokuna pura llank'ay tinkuy. Ayllukunapa minga tupayninwan, redman tukusqa.
- **[wawa](wawa/README.md)** — *wawa* (criatura mosoq). Pacha musuq kunallan sistema operativo (`wawa-kernel`, `wawa-boot`, `wawa-fs`, `apps/`). POSIX → BLAKE3; filesystem content-addressed DAG hina; gaming-grade (AOT WASM + GPU + frame pacing cooperativo).
- **[wawa-explorer](wawa-explorer/README.md)** — Wawapa DAG ñawi host-pi: `.img` ñawinchaq, Akasha protocolo raw sockets ukhuwan rimaq, sach'ata Llimphi ñawichaspa rikuchin.

## Manifesto

> **Saphiqa upalla sayachin.**
> Imayhanaqkitaqmi ch'unlla unaymi: mana rikuchikuq sumaq llank'aqtin. Allin kernel pi runa mana riqsiq.
>
> 1. **Mana yanqallamanta deps saphipi.** Sapanka `ukupacha` crate Cargo.toml lineanta sutilla kamachisqa.
> 2. **Content-addressed sapanmanta.** BLAKE3 sutimi — bytes chiqaqmi, sutikuna willaymi.
> 3. **Runaqa manan kernel-paq cliente.** Kernel-paq cliente operador. Sumaq-runa herramientakunaqa `02_ruway`-pi tiyanku.
> 4. **Qillqayoq imaykita iskay chunka wata hawamanta arkeólogo ñawichanan hina.** SDDkuna, IMARAYKUkuna, qillqasqa razónkuna — chaykunamantallam autorpa qhipanninpi imapas kawsaqtin tarikun.
