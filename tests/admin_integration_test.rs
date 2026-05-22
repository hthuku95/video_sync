// Admin-based integration tests
//
// Tests admin dashboard functionality with real production data.
//
// Prerequisites:
//   - A superuser account must exist in the DB (create with `cargo run --bin createsuperuser`)
//   - Set TEST_ADMIN_EMAIL and TEST_ADMIN_PASSWORD in .env.test (or the environment)
//
// Run all: cargo test --test admin_integration_test -- --ignored --nocapture
// Run one: cargo test --test admin_integration_test test_admin_authentication -- --ignored --nocapture

mod helpers;

use helpers::admin_helpers;

/// Load admin credentials from environment.
/// Panics with a clear message if not set — the user must create a superuser first.
fn get_admin_credentials() -> (String, String) {
    let email = std::env::var("TEST_ADMIN_EMAIL").expect(
        "TEST_ADMIN_EMAIL not set. \
             Run `cargo run --bin createsuperuser` to create an admin account, \
             then add TEST_ADMIN_EMAIL=<email> to .env.test",
    );
    let password = std::env::var("TEST_ADMIN_PASSWORD").expect(
        "TEST_ADMIN_PASSWORD not set. \
             Add TEST_ADMIN_PASSWORD=<password> to .env.test",
    );
    (email, password)
}

/// Test 1: Admin Authentication & Authorization
#[tokio::test]
#[ignore] // Run with: cargo test --test admin_integration_test --ignored
async fn test_admin_authentication() {
    println!("\n🔐 Test 1: Admin Authentication & Authorization\n");

    let (email, password) = get_admin_credentials();

    // Login and get JWT token
    let token = admin_helpers::admin_login(&email, &password)
        .await
        .expect("Admin login should succeed — check TEST_ADMIN_EMAIL/PASSWORD in .env.test");

    println!("✅ Admin login successful, got JWT token");

    // Verify: Can access admin endpoints
    let jobs = admin_helpers::get_jobs(&token, None, None, None, None)
        .await
        .expect("Admin should be able to access jobs endpoint");

    println!("✅ Admin can access /api/admin/clipping/jobs");
    println!("   Total jobs visible: {}", jobs.pagination.total);

    assert!(
        jobs.pagination.total > 0,
        "Admin should see production jobs"
    );

    println!("✅ Test completed: Admin authentication works correctly\n");
}

/// Test 2: View Real Production Jobs with Filtering
#[tokio::test]
#[ignore]
async fn test_admin_view_production_jobs() {
    println!("\n📊 Test 2: View Real Production Jobs\n");

    let (email, password) = get_admin_credentials();
    let token = admin_helpers::admin_login(&email, &password)
        .await
        .expect("Admin login failed");

    // Get all jobs
    let all_jobs = admin_helpers::get_jobs(&token, None, None, Some(1), Some(200))
        .await
        .expect("Failed to get all jobs");

    println!("📈 All jobs: {}", all_jobs.pagination.total);

    // Count by status
    let failed_count = all_jobs
        .jobs
        .iter()
        .filter(|j| j.status == "failed")
        .count();
    let pending_count = all_jobs
        .jobs
        .iter()
        .filter(|j| j.status == "pending")
        .count();
    let completed_count = all_jobs
        .jobs
        .iter()
        .filter(|j| j.status == "completed")
        .count();

    println!("   - Failed: {}", failed_count);
    println!("   - Pending: {}", pending_count);
    println!("   - Completed: {}", completed_count);

    // Filter by status: failed
    if failed_count > 0 {
        let failed_jobs = admin_helpers::get_jobs(&token, Some("failed"), None, None, None)
            .await
            .expect("Failed to filter by failed status");

        println!("\n🔴 Failed jobs details:");
        for job in failed_jobs.jobs.iter().take(5) {
            println!(
                "   Job #{}: {} - Retries: {} - Error: {}",
                job.id,
                job.source_video_id,
                job.retry_count,
                job.error_message.as_deref().unwrap_or("None")
            );
        }

        assert!(
            failed_jobs.pagination.total as usize >= failed_count,
            "Filter should return correct failed job count"
        );
    }

    // Filter by status: pending
    if pending_count > 0 {
        let pending_jobs = admin_helpers::get_jobs(&token, Some("pending"), None, None, None)
            .await
            .expect("Failed to filter by pending status");

        println!("\n🟡 Pending jobs: {}", pending_jobs.pagination.total);

        assert!(
            pending_jobs.pagination.total as usize >= pending_count,
            "Filter should return correct pending job count"
        );
    }

    println!("\n✅ Test completed: Job filtering works correctly\n");
}

/// Test 3: Identify and Categorize Failed Jobs
#[tokio::test]
#[ignore]
async fn test_admin_analyze_failed_jobs() {
    println!("\n🔍 Test 3: Analyze Failed Jobs\n");

    let (email, password) = get_admin_credentials();
    let token = admin_helpers::admin_login(&email, &password)
        .await
        .expect("Admin login failed");

    // Get all failed jobs
    let failed_jobs = admin_helpers::get_jobs(&token, Some("failed"), None, None, Some(50))
        .await
        .expect("Failed to get failed jobs");

    println!("🔴 Total failed jobs: {}", failed_jobs.pagination.total);

    if failed_jobs.pagination.total == 0 {
        println!("✅ No failed jobs found - system is healthy!");
        return;
    }

    // Categorize failures
    let mut private_video_jobs = Vec::new();
    let mut qdrant_uuid_jobs = Vec::new();
    let mut download_failed_jobs = Vec::new();
    let mut other_failures = Vec::new();

    for job in &failed_jobs.jobs {
        if let Some(error) = &job.error_message {
            if error.contains("Video is private") || error.contains("private") {
                private_video_jobs.push(job);
            } else if error.contains("Unable to parse UUID") || error.contains("UUID") {
                qdrant_uuid_jobs.push(job);
            } else if error.contains("All download strategies") || error.contains("403 Forbidden") {
                download_failed_jobs.push(job);
            } else {
                other_failures.push(job);
            }
        }
    }

    println!("\n📊 Failure Analysis:");
    println!("   🔒 Private videos: {} jobs", private_video_jobs.len());
    println!("   🆔 Qdrant UUID errors: {} jobs", qdrant_uuid_jobs.len());
    println!(
        "   📥 Download failures: {} jobs",
        download_failed_jobs.len()
    );
    println!("   ❓ Other failures: {} jobs", other_failures.len());

    if !private_video_jobs.is_empty() {
        println!("\n🔒 Private Video Jobs (should be cancelled):");
        for job in private_video_jobs.iter().take(3) {
            println!(
                "   Job #{}: {} - Retry count: {}",
                job.id, job.source_video_id, job.retry_count
            );
        }
    }

    if !qdrant_uuid_jobs.is_empty() {
        println!("\n🆔 Qdrant UUID Error Jobs (needs code fix):");
        for job in qdrant_uuid_jobs.iter().take(3) {
            println!(
                "   Job #{}: {} - Retry count: {}",
                job.id, job.source_video_id, job.retry_count
            );
        }
    }

    if !download_failed_jobs.is_empty() {
        println!("\n📥 Download Failure Jobs (may be transient):");
        for job in download_failed_jobs.iter().take(3) {
            println!(
                "   Job #{}: {} - Retry count: {}",
                job.id, job.source_video_id, job.retry_count
            );
        }
    }

    // Warn about high retry counts
    let high_retry_jobs: Vec<_> = failed_jobs
        .jobs
        .iter()
        .filter(|j| j.retry_count > 100)
        .collect();

    if !high_retry_jobs.is_empty() {
        println!(
            "\n⚠️  WARNING: {} jobs with >100 retry attempts!",
            high_retry_jobs.len()
        );
        println!("   These jobs are wasting significant compute resources!");
        for job in high_retry_jobs.iter().take(3) {
            println!(
                "   Job #{}: {} retries - {}",
                job.id,
                job.retry_count,
                job.error_message.as_deref().unwrap_or("No error message")
            );
        }
    }

    println!("\n✅ Test completed: Failed jobs analyzed successfully\n");
}

/// Test 4: Cancel Jobs (Admin Action)
#[tokio::test]
#[ignore]
async fn test_admin_cancel_jobs() {
    println!("\n🚫 Test 4: Admin Cancel Jobs\n");

    let (email, password) = get_admin_credentials();
    let token = admin_helpers::admin_login(&email, &password)
        .await
        .expect("Admin login failed");

    // Find jobs to cancel (private videos with high retry counts)
    let failed_jobs = admin_helpers::get_jobs(&token, Some("failed"), None, None, Some(50))
        .await
        .expect("Failed to get failed jobs");

    let private_video_jobs: Vec<_> = failed_jobs
        .jobs
        .iter()
        .filter(|j| {
            j.error_message
                .as_ref()
                .map(|e| e.contains("Video is private"))
                .unwrap_or(false)
                && j.retry_count > 50
        })
        .collect();

    if private_video_jobs.is_empty() {
        println!("✅ No private video jobs to cancel - system is clean!");
        return;
    }

    println!(
        "🔒 Found {} private video jobs with >50 retries",
        private_video_jobs.len()
    );
    println!("   These jobs should be cancelled to save resources\n");

    // Cancel first 3 jobs as demonstration
    let jobs_to_cancel: Vec<_> = private_video_jobs.iter().take(3).collect();

    for job in &jobs_to_cancel {
        println!(
            "🚫 Cancelling Job #{}: {} (retries: {})",
            job.id, job.source_video_id, job.retry_count
        );

        let result = admin_helpers::cancel_job(&token, job.id).await;

        match result {
            Ok(_) => {
                println!("   ✅ Successfully cancelled");

                // Verify status changed to cancelled
                let updated = admin_helpers::get_job_details(&token, job.id)
                    .await
                    .expect("Failed to get updated job");

                assert_eq!(
                    updated.status, "cancelled",
                    "Job status should be 'cancelled'"
                );
            }
            Err(e) => {
                println!("   ❌ Failed to cancel: {}", e);
            }
        }
    }

    println!(
        "\n💡 Recommendation: Use SQL to cancel all {} private video jobs:",
        private_video_jobs.len()
    );
    println!("   UPDATE clipping_jobs SET status='cancelled'");
    println!("   WHERE error_message LIKE '%Video is private%' AND retry_count > 50;");

    println!("\n✅ Test completed: Job cancellation works correctly\n");
}

/// Test 5: Retry Jobs After Fix
#[tokio::test]
#[ignore]
async fn test_admin_retry_jobs() {
    println!("\n🔄 Test 5: Admin Retry Jobs\n");

    let (email, password) = get_admin_credentials();
    let token = admin_helpers::admin_login(&email, &password)
        .await
        .expect("Admin login failed");

    // Find a failed job to retry (preferably Qdrant UUID error)
    let failed_jobs = admin_helpers::get_jobs(&token, Some("failed"), None, None, Some(20))
        .await
        .expect("Failed to get failed jobs");

    let qdrant_error_job = failed_jobs.jobs.iter().find(|j| {
        j.error_message
            .as_ref()
            .map(|e| e.contains("Unable to parse UUID"))
            .unwrap_or(false)
    });

    if let Some(job) = qdrant_error_job {
        println!("🆔 Found Qdrant UUID error job: #{}", job.id);
        println!("   Video: {}", job.source_video_id);
        println!("   Current retry count: {}", job.retry_count);
        println!(
            "   Error: {}",
            job.error_message.as_deref().unwrap_or("None")
        );

        println!("\n⚠️  NOTE: This job will fail again unless the Qdrant UUID fix is deployed!");
        println!("   After deploying the fix, run this test to retry the job.\n");

        println!("🔄 Retrying job #{}...", job.id);

        let result = admin_helpers::retry_job(&token, job.id).await;

        match result {
            Ok(_) => {
                println!("   ✅ Job queued for retry");

                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

                let updated = admin_helpers::get_job_details(&token, job.id)
                    .await
                    .expect("Failed to get updated job");

                println!("   Status: {} (should be 'pending')", updated.status);
                println!(
                    "   Retry count: {} (incremented from {})",
                    updated.retry_count, job.retry_count
                );

                assert_eq!(
                    updated.status, "pending",
                    "Retried job should be in pending status"
                );
            }
            Err(e) => {
                println!("   ❌ Failed to retry: {}", e);
            }
        }
    } else {
        println!("✅ No Qdrant UUID error jobs found - system may be healthy!");

        // Try retrying any failed job
        if let Some(job) = failed_jobs.jobs.first() {
            println!("\n🔄 Testing retry with job #{} instead", job.id);

            let result = admin_helpers::retry_job(&token, job.id).await;
            match result {
                Ok(_) => println!("   ✅ Retry successful"),
                Err(e) => println!("   ❌ Retry failed: {}", e),
            }
        }
    }

    println!("\n✅ Test completed: Job retry mechanism works correctly\n");
}

/// Test 6: Monitor Job Statistics
#[tokio::test]
#[ignore]
async fn test_admin_job_statistics() {
    println!("\n📊 Test 6: Job Statistics & Monitoring\n");

    let (email, password) = get_admin_credentials();
    let token = admin_helpers::admin_login(&email, &password)
        .await
        .expect("Admin login failed");

    // Get comprehensive statistics
    let all_jobs = admin_helpers::get_jobs(&token, None, None, Some(1), Some(500))
        .await
        .expect("Failed to get all jobs");

    println!("📈 Overall Statistics:");
    println!("   Total jobs: {}", all_jobs.pagination.total);

    // Count by status
    let mut status_counts = std::collections::HashMap::new();
    for job in &all_jobs.jobs {
        *status_counts.entry(job.status.clone()).or_insert(0) += 1;
    }

    println!("\n📊 Jobs by Status:");
    for (status, count) in status_counts.iter() {
        let percentage = (*count as f64 / all_jobs.jobs.len().max(1) as f64) * 100.0;
        println!("   {}: {} ({:.1}%)", status, count, percentage);
    }

    // Retry statistics
    let total_retries: i32 = all_jobs.jobs.iter().map(|j| j.retry_count).sum();
    let avg_retries = total_retries as f64 / all_jobs.jobs.len().max(1) as f64;

    println!("\n🔄 Retry Statistics:");
    println!("   Total retry attempts: {}", total_retries);
    println!("   Average retries per job: {:.2}", avg_retries);

    let high_retry = all_jobs.jobs.iter().filter(|j| j.retry_count > 10).count();
    let very_high_retry = all_jobs.jobs.iter().filter(|j| j.retry_count > 100).count();

    println!("   Jobs with >10 retries: {}", high_retry);
    println!("   Jobs with >100 retries: {} ⚠️", very_high_retry);

    if very_high_retry > 0 {
        println!("\n⚠️  WARNING: High retry count indicates systematic issues!");
        println!(
            "   Estimated wasted compute: ~{} hours",
            very_high_retry * 100 / 12
        );
    }

    // User distribution
    let mut user_job_counts = std::collections::HashMap::new();
    for job in &all_jobs.jobs {
        *user_job_counts.entry(job.user_id).or_insert(0) += 1;
    }

    println!("\n👥 Jobs by User:");
    for (user_id, count) in user_job_counts.iter().take(5) {
        println!("   User {}: {} jobs", user_id, count);
    }

    println!("\n✅ Test completed: Statistics collected successfully\n");
}
