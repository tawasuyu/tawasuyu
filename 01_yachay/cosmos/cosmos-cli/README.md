# cosmos-cli

> CLI de [cosmos](../README.md).

Comandos:

```sh
cosmos-cli download              # DE files + catálogos
cosmos-cli when "venus rises"    # próximo rise de venus
cosmos-cli where mars            # posición actual de marte
cosmos-cli eclipses --next 5     # próximos 5 eclipses
cosmos-cli sky --time now        # snapshot del cielo
cosmos-cli validate              # corre el regression harness
```

## Deps

- Todos los `cosmos-*` core
- `clap`, `serde_json`
