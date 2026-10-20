# Runbook: rotate the signing key

> [!CAUTION]
> Rotating the key logs out every active session.

## Before you start

- [ ] Announce the maintenance window.
- [ ] Check that the backup job finished: [OK](adf:status/green)
- [x] Read the [key management policy](https://example.com/policy).

## Steps

1. Generate a new key:

   ```sh
   keytool generate --algorithm ed25519 --out new.key
   ```

2. Upload it as the *secondary* key.
3. Wait for **all** replicas to report the new key:

   | Replica | Region | Ready |
   |---------|--------|:-----:|
   | r1 | eu | yes |
   | r2 | us | yes |

4. Promote the secondary key.

<details>
<summary>Rollback</summary>

Promote the previous key again, then remove the new one:

```sh
keytool promote --key previous
```

</details>

Last reviewed [2026-09-30](adf:date).
