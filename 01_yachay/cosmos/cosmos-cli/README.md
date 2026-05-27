# cosmos-cli

> CLI of [cosmos](../README.md).

Commands:

```sh
cosmos-cli download              # DE files + catalogs
cosmos-cli when "venus rises"    # next venus rise
cosmos-cli where mars            # current mars position
cosmos-cli eclipses --next 5     # next 5 eclipses
cosmos-cli sky --time now        # sky snapshot
cosmos-cli validate              # runs the regression harness
```

## Deps

- All `cosmos-*` core
- `clap`, `serde_json`
