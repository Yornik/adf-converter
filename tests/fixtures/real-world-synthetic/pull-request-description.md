## Summary

Adds retry with exponential backoff to the webhook sender.

## Changes

| File | Change |
|------|--------|
| `sender.rs` | retry loop with jitter |
| `config.rs` | new `max_retries` setting (default `5`) |
| `README.md` | documents the setting |

## Checklist

- [x] Tests added
- [x] Docs updated
- [ ] Changelog entry

## Notes for reviewers

The backoff is capped at **30 s**; see the discussion in https://example.com/rfcs/17.

Reviewers: [@Ana](adf:mention/abc-123) [@Bo](adf:mention/def-456)
