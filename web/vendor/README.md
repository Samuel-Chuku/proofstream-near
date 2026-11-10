# Vendored code

| File | Source | sha256 |
|---|---|---|
| `near-connect-0.12.0.mjs` | npm `@hot-labs/near-connect@0.12.0`, tarball sha256 `b5cd265f70e76c1fa3aeba5fca5640939fb577f7a385ca96eb44ead8f8898e4e`, wrapped into one ES module by `../tools/bundle-near-connect.mjs` (no bundler, no other packages) | `1ba2177b4106dd7a31087937f8f5b19ef3e05bd4c70a31cde38ff37ed9524ee6` |

It is the only third-party code on the site, and it loads only when someone connects a wallet.

Rebuild it and compare:

```bash
npm pack @hot-labs/near-connect@0.12.0
sha256sum hot-labs-near-connect-0.12.0.tgz        # b5cd265f…
tar -xzf hot-labs-near-connect-0.12.0.tgz
node web/tools/bundle-near-connect.mjs package/build > /tmp/near-connect.mjs
sha256sum /tmp/near-connect.mjs web/vendor/near-connect-0.12.0.mjs
```
