title:: Atlas/Architecture
type:: doc

- Offline first: every write lands locally, sync is a background concern #atlas/sync
- Retry policy for the sync worker
  - ```rust
    fn backoff(attempt: u32) -> Duration {
        let secs = 2u64.pow(attempt.min(6));
        Duration::from_secs(secs) + jitter()
    }
    ```
- Open question from [[@Priya Shah]]: do we need ==conflict banners== on Android?
- Storage choice and its trade-offs live on [[Project Atlas]]
