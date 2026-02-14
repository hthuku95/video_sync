# Integration Tests for VideoSync Clipping System

Comprehensive integration tests that validate the entire clipping workflow from job creation to YouTube upload.

## Test Coverage

### 1. Complete E2E Workflow (`test_complete_clipping_workflow`)
- **Duration**: 5-8 minutes
- **What it tests**:
  - Job creation and claiming
  - YouTube video download (5-tier fallback)
  - Video vectorization (Qdrant storage)
  - AI clip extraction (Claude Vision)
  - YouTube upload and verification
- **Expected outcome**: Job completes successfully with clips extracted

### 2. Atomic Job Claiming (`test_job_claiming_atomicity`)
- **Duration**: 1-2 minutes
- **What it tests**:
  - PostgreSQL FOR UPDATE SKIP LOCKED prevents race conditions
  - Multiple workers claim different jobs simultaneously
  - No duplicate processing occurs
- **Expected outcome**: 3 workers claim 3 different jobs from pool of 5

### 3. Download Fallback System (`test_download_strategies_fallback`)
- **Duration**: 3-5 minutes
- **What it tests**:
  - 5-tier download strategy cascading (Apify → rustube → yt-dlp → rust-yt-downloader → rusty_ytdl)
  - Recovery from transient failures
  - Logging of fallback attempts
- **Expected outcome**: Video downloads successfully via any available strategy

### 4. Stuck Job Detection (`test_stuck_job_detection`)
- **Duration**: < 1 minute
- **What it tests**:
  - Jobs stuck in intermediate states (analyzing > 60 min)
  - Automatic timeout detection
  - Reset to failed status with error message
- **Expected outcome**: Stuck job detected and marked as failed

### 5. Auto-Retry Failed Jobs (`test_auto_retry_failed_jobs`)
- **Duration**: < 1 minute
- **What it tests**:
  - Failed jobs retry after 5 minutes
  - Retry count incremented
  - Error cleared on retry
  - Time window enforcement (6 hours)
- **Expected outcome**: Failed job reset to pending with incremented retry_count

## Setup

### Prerequisites
1. **Database**: Neon PostgreSQL (shared with production, isolated by test user ID)
2. **Environment**: Copy `.env` to `.env.test` and configure test settings
3. **API Keys**: Voyage AI, Gemini, Apify, Claude (optional for full tests)

### Configuration

Edit `.env.test`:
```bash
# Required
DATABASE_URL=postgresql://...

# Optional (for full E2E tests)
VOYAGEAI_API_KEY=your_key
GEMINI_API_KEY=your_key
APIFY_API_KEY=your_key
CLAUDE_API_KEY=your_key
QDRANT_URL=your_url
QDRANT_API_KEY=your_key
```

## Running Tests

### Method 1: Cargo Test (Recommended)
```bash
# Run all integration tests
cargo test --test clipping_integration_test --ignored -- --test-threads=1

# Run specific test
cargo test --test clipping_integration_test test_job_claiming_atomicity --ignored

# With verbose logging
RUST_LOG=info cargo test --test clipping_integration_test --ignored -- --nocapture
```

### Method 2: Custom Test Runner
```bash
# Run all tests with detailed output
cargo run --bin run_clipping_integration_tests
```

### Method 3: Individual Test Execution
```bash
# Run just the atomicity test
cargo test --test clipping_integration_test test_job_claiming_atomicity --ignored -- --nocapture
```

## Test Data Isolation

- **Test User ID**: `-1` (negative to avoid conflicts with real users)
- **Test Channels**: Prefixed with `TEST_` and timestamped
- **Automatic Cleanup**: `Drop` trait ensures test data is deleted even on panic
- **Manual Cleanup**: Call `ctx.cleanup().await` explicitly

## Expected Test Costs

### Per Test Run
- Apify downloads: $0.02-0.05
- Gemini embeddings: $0.01-0.02
- Voyage AI embeddings: $0.01-0.02
- Claude Vision: $0.02-0.03
- **Total: ~$0.06-0.13 per test**

### Full Suite
- ~$0.50-1.00 per complete test run
- Monthly dev: $10-20 (20-40 test runs)

## Troubleshooting

### Database Connection Errors
```
Error: connection refused
```
**Solution**: Check `DATABASE_URL` in `.env.test` matches Neon connection string

### Test Timeouts
```
Job should complete within X minutes
```
**Solution**:
- Check Render worker is running
- Verify API keys are valid
- Increase timeout in test code if on slow connection

### Cleanup Failures
```
Failed to cleanup test data
```
**Solution**: Manually delete test data:
```sql
DELETE FROM extracted_clips WHERE job_id IN (SELECT id FROM clipping_jobs WHERE linkage_id IN (SELECT id FROM youtube_channel_linkages WHERE user_id = -1));
DELETE FROM clipping_jobs WHERE linkage_id IN (SELECT id FROM youtube_channel_linkages WHERE user_id = -1);
DELETE FROM youtube_channel_linkages WHERE user_id = -1;
DELETE FROM users WHERE id = -1;
```

### Missing Test Videos
```
Video not found or unavailable
```
**Solution**: Update `test_videos` constants in `helpers/test_youtube.rs` with accessible public videos

## CI/CD Integration

### GitHub Actions Example
```yaml
name: Integration Tests

on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v2
      - uses: actions-rs/toolchain@v1
        with:
          toolchain: stable

      - name: Run integration tests
        env:
          DATABASE_URL: ${{ secrets.TEST_DATABASE_URL }}
          VOYAGEAI_API_KEY: ${{ secrets.VOYAGEAI_API_KEY }}
          GEMINI_API_KEY: ${{ secrets.GEMINI_API_KEY }}
        run: cargo test --test clipping_integration_test --ignored
```

## Maintenance

### Adding New Tests
1. Create test function in `tests/integration/clipping_integration_test.rs`
2. Add `#[tokio::test]` and `#[ignore]` attributes
3. Use `TestContext` for database setup/cleanup
4. Use assertion helpers from `helpers/assertions.rs`
5. Document expected duration and cost

### Updating Test Data
- Update `test_videos` in `helpers/test_youtube.rs`
- Ensure videos are public domain or permissive license
- Prefer short videos (< 5 min) to reduce test time

### Performance Benchmarking
```bash
# Run with timing
time cargo test --test clipping_integration_test test_complete_clipping_workflow --ignored -- --nocapture
```

## Best Practices

1. **Run tests sequentially** (`--test-threads=1`) to avoid database conflicts
2. **Use `.env.test`** to separate test config from production
3. **Always cleanup** test data to avoid bloating database
4. **Monitor costs** - integration tests use real APIs
5. **Update test videos** if URLs become unavailable
6. **Document failures** - create issues for flaky tests

## Support

For issues with integration tests:
1. Check GitHub issues: https://github.com/hthuku95/video_sync/issues
2. Review test logs with `RUST_LOG=debug`
3. Verify all prerequisites are met
4. Check production worker is running on Render
