# Licencia

La mayor parte de **gioser** se publica con licencia dual, a tu elección:

- **Apache License 2.0** ([`LICENSE-APACHE`](LICENSE-APACHE)), o
- **MIT** ([`LICENSE-MIT`](LICENSE-MIT)).

`SPDX-License-Identifier: MIT OR Apache-2.0`

## Excepción — núcleos bajo MPL-2.0

Algunos crates de base que cruzan la frontera kernel/userspace o viajan por
disco direccionado por contenido se publican bajo **Mozilla Public License 2.0**
([`LICENSE-MPL`](LICENSE-MPL)):

- `shared/format`
- `shared/forth-emisor`
- `shared/foreign-fs`
- `03_ukupacha/wawa` (`wawa`, `wawa-kernel`, `wawa-fs`)

Cada crate declara su licencia en su `Cargo.toml` (`license = "…"`); ese campo
es la fuente autoritativa cuando haya duda.

## Contribuciones

Salvo que declares lo contrario, toda contribución que envíes para inclusión en
un crate con licencia dual queda licenciada como `MIT OR Apache-2.0`, sin
términos ni condiciones adicionales.
