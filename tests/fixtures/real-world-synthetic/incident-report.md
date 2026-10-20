## ✍️ **Summary**

::: cell
Between 08:14 and 09:26 UTC, newly added documents did not appear in search results for customers in `eu-central`. No data was lost.

The indexer ran out of memory after a release increased its batch size.
:::

## 📡 **Detection**

::: cell
- The index lag alert fired four minutes after the first restart.
- Support received no customer reports before the alert.
:::

## 👥 **Impact**

::: cell
Affected:

- About **12%** of search requests returned stale results.
  - New documents were missing for up to 72 minutes.
- No requests failed.
:::

## 🧭 **Response**

::: cell
The on-call engineer followed the runbook:

1. Acknowledge the page and open the incident channel.
2. Roll back the most recent deploy.
3. Lower the batch size with the `indexer.batch_size` flag.
:::

## 🗓️ **Timeline**

| **Time (UTC)** | **Event** |
|---|---|
| 2026-03-02 08:14 | Index lag alert fires for `kestrel-indexer` in `eu-central`. |
| 2026-03-02 08:16 | On-call engineer acknowledges the page. |
| 2026-03-02 08:21 | Dashboards show the `ingest` queue depth growing steadily. |
| 2026-03-02 08:25 | Worker pods report `OOMKilled` restarts. |
| 2026-03-02 08:31 | Incident channel opened; status set to investigating. |
| 2026-03-02 08:38 | Recent deploy `v4.18.0` identified as a candidate cause. |
| 2026-03-02 08:44 | Rollback of `kestrel-indexer` to `v4.17.2` started. |
| 2026-03-02 08:52 | Rollback finished; restarts continue at a lower rate. |
| 2026-03-02 09:03 | Heap profile shows a large batch buffer in `BatchWriter`. |
| 2026-03-02 09:10 | Batch size lowered from `5000` to `500` via config flag. |
| 2026-03-02 09:18 | Restarts stop; queue depth starts to fall. |
| 2026-03-02 09:26 | Search results for new documents appear again. |
| 2026-03-02 09:40 | Customer support notified of the recovery. |
| 2026-03-02 09:55 | Queue depth back under the alert threshold. |
| 2026-03-02 10:05 | Backfill job `reindex-recent` started for the gap. |
| 2026-03-02 10:40 | Backfill reaches 50 percent. |
| 2026-03-02 11:15 | Backfill completes; spot checks pass. |
| 2026-03-02 11:20 | Status set to monitoring. |
| 2026-03-02 12:00 | No further restarts observed. |
| 2026-03-02 13:30 | Config flag change made permanent in `values.yaml`. |
| 2026-03-02 14:10 | Fix for unbounded buffer opened as `PR-1287`. |
| 2026-03-02 15:45 | Fix reviewed and merged. |
| 2026-03-02 16:20 | `v4.18.1` deployed with the fix. |
| 2026-03-02 17:00 | Incident resolved. |
| 2026-03-03 09:00 | Post-incident review scheduled. |

## 🔍 **Root cause**

::: cell
1. Release `v4.18.0` raised the default batch size to `5000`.

   The `BatchWriter` buffers a whole batch in memory before writing.

   Large documents made each batch exceed the memory limit.

2. The memory limit of the worker pods was not changed.
:::

## 🛠️ **Remediation**

::: cell
Completed:

Follow-up actions are tracked in the backlog.

- Bound the buffer by bytes as well as by count.

  This landed in `v4.18.1`.

  The new limit is configurable.

- Add a memory-usage panel to the indexer dashboard.
:::

## 📚 **Lessons learned**

::: cell
- Configuration defaults deserve the same review as code.
- Rollbacks alone do not undo a config change that shipped with them.
:::

## ✅ **Action items**

::: cell
1. Add a load test with large documents. Owner: search team.
2. Alert on worker restarts, not only on index lag. Owner: on-call rotation.
:::

> [!WARNING]
> This report describes a fictional service and is used as a test fixture only.
