# iniy-wiki

> Wikipedia/MediaWiki crawler/parser for [iniy](../README.md).

Reads official dumps (XML/SQL) or calls the MediaWiki API for specific articles. Normalizes wikitext to an [`iniy-extract`](../iniy-extract/README.md)-ingestable `Documento`. Respects `robots.txt` and rate limits when crawling live; clear preference for local dumps.

## Usage

```sh
# parse local XML dump
iniy-cli wiki-ingest --dump enwiki-latest-pages-articles.xml.bz2

# fetch single article
iniy-cli wiki-fetch --title "Quantum mechanics"
```

## Deps

- [`iniy-ingest`](../iniy-ingest/README.md)
- `parse_wiki_text`, `reqwest`
