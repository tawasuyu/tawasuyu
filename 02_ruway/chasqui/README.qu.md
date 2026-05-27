<!-- Quechua (Cusco/Collao). Revisión bienvenida. -->

# chasqui

> `chasqui` (runa-simi: *Inka ñanninpa mensahero*). Mensaje broker + tipo bus.

Monorepupa sapan nervio. Aplikacionkuna tipasqa topics-pi publish + subscribe ruwanku; brokerqa rutéo + waqaychaq. `nous` backend, iskay implementaciones: `mock` (in-process tests-paq) + `real` (binario TCP). Sapanka mensaje schema-yuq, mana yachaqtin chaski-fail.

## Churay

```sh
cargo run --release -p chasqui-broker
cargo run --release -p chasqui-broker-explorer-llimphi
cargo run --release -p chasqui-explorer-llimphi
```

## Tinkuy

- **Linux / macOS / Windows** — broker + cliente Rust naturalwan.
- **Wawa** — broker kernel apps hina.
- TCP localhost default; Unix sockets opcional.

## Crateskuna

| Crate | Ima ruwan |
|---|---|
| [`chasqui-core`](chasqui-core/README.md) | Topic, Mensaje, Schema, Subscripción. |
| [`chasqui-broker`](chasqui-broker/README.md) | Broker binario. |
| [`chasqui-nous`](chasqui-nous/README.md) | Transporte trait. |
| [`chasqui-nous-mock`](chasqui-nous-mock/README.md) | In-process transporte. |
| [`chasqui-nous-real`](chasqui-nous-real/README.md) | Binario TCP/Unix transporte. |
| [`chasqui-card`](chasqui-card/README.md) | Escritorio card. |
| [`chasqui-broker-explorer-llimphi`](chasqui-broker-explorer-llimphi/README.md) | Topics + subscriptores UI. |
| [`chasqui-explorer-llimphi`](chasqui-explorer-llimphi/README.md) | Kawsaq mensaje log UI. |

## Yuyaykunaq

- **Schema-ñawpaq.** Mana schema mana mensaje.
- **Waqaychay opt-in** topic-pi; ephemeris topics ukhupi kawsanku.
- **Mana Kafka.** Monorepupaq, mana planetapaq.
