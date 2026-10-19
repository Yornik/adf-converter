# Release 2.4.0

## Highlights

- **Faster imports**: large CSV files import about twice as fast.
- New `--dry-run` flag for `sync`.
- Dark mode for the settings page ([#412](https://example.com/issues/412)).

## Breaking changes

> [!IMPORTANT]
> The `legacy_auth` option was removed. Use `auth.mode = "token"` instead.

## Fixed

1. Dates before 1970 are displayed correctly.
2. The export no longer drops the last row.
3. Emoji in file names 🎉 are preserved.

## Upgrade

```sh
tool upgrade --to 2.4.0
tool migrate --check
```

Thanks to [@Sam Rivera](adf:mention/557058:0a1b2c3d-4e5f-6789-abcd-ef0123456789) for the import work.
