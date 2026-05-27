# iniy-wiki

> Crawler/parser para Wikipedia/MediaWiki en [iniy](../README.md).

Lee dumps oficiales (XML/SQL) o llama a la API de MediaWiki para artículos específicos. Normaliza el wikitext a un `Documento` ingestable por [`iniy-extract`](../iniy-extract/README.md). Respeta `robots.txt` y rate limits cuando crawlea live; preferencia clara por dumps locales.

## Uso

```sh
# parsear dump XML local
iniy-cli wiki-ingest --dump enwiki-latest-pages-articles.xml.bz2

# fetch single article
iniy-cli wiki-fetch --title "Quantum mechanics"
```

## Deps

- [`iniy-ingest`](../iniy-ingest/README.md)
- `parse_wiki_text`, `reqwest`
