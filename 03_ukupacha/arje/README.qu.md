<!-- Quechua (Cusco/Collao). Revisión bienvenida. -->

# arje

> `arje` (griego *ἀρχή*: paqarisqa, qallariy). Bootloader + sistemata kawsachiq.

`arje` "ENCENDER ñit'irqanki"-manta "kernel puriqshanña"-kama: muhukuna, huñuy, churay, kawsaq sistemata hap'iy, sapan kernel, minimo red, kamachiy + auditoría, CAS, snapshots, soma, WASM init.

## Churay

```sh
cargo run --release -p arje-packager -- build --target iso
cargo run --release -p arje-installer
cargo run --release -p arje-absorb -- /path/to/system
```

## Tinkuy

- **Linux x86_64** — ñawpaq target.
- **aarch64** — `arje-kernel` chinka-suyay.
- **Wawa** — `arje` `wawa-kernel`-pa natural bootloader.

Crateskuna [README.md](README.md)-pi.

## Yuyaykunaq

- **`arje` qallarinapaq, mana sistema kamachiy puriqpaqchu**. Chayqa `wawa-kernel`-pa.
- `absorb` mana imapas tikran: read-only host-pi, mana paqsi objeto kutichiq.
- Sapanka crate sapanka Cargo.toml linea sutilla — saphi mana yanqalla deps haykachinchu.
