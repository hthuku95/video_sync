use aws_config::BehaviorVersion;
use serde_json::Value;

pub struct SQSClient {
    client: aws_sdk_sqs::Client,
    queue_url: String,
}

impl SQSClient {
    pub async fn new(queue_url: String) -> Result<Self, String> {
        let config = aws_config::defaults(BehaviorVersion::latest())
            .region(aws_config::Region::new("us-east-1"))
            .load()
            .await;
        let client = aws_sdk_sqs::Client::new(&config);
        Ok(Self { client, queue_url })
    }

    pub async fn enqueue(&self, job_id: i32, job_type: &str, payload: Value) -> Result<(), String> {
        let body = serde_json::json!({
            "job_id": job_id,
            "job_type": job_type,
            "payload": payload,
        });

        self.client
            .send_message()
            .queue_url(&self.queue_url)
            .message_body(body.to_string())
            .send()
            .await
            .map_err(|e| format!("SQS enqueue failed: {}", e))?;

        Ok(())
    }
}
