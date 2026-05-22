// Utility to fix Qdrant collection dimension mismatch
use qdrant_client::qdrant::{CreateCollectionBuilder, Distance, VectorParamsBuilder};
use qdrant_client::Qdrant;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🔧 Fixing Qdrant dimension mismatch...");

    let qdrant_url = std::env::var("QDRANT_URL").unwrap_or_else(|_| {
        "https://18635ac0-f6b3-43b3-9255-54a553f6c2fb.us-west-1-0.aws.cloud.qdrant.io:6334"
            .to_string()
    });
    let qdrant_api_key =
        std::env::var("QDRANT_API_KEY").expect("QDRANT_API_KEY environment variable not set");

    let client = Qdrant::from_url(&qdrant_url)
        .api_key(qdrant_api_key)
        .build()?;

    println!("📡 Connected to Qdrant");

    // Delete old collection
    println!("🗑️  Deleting old 'chat_memory' collection...");
    match client.delete_collection("chat_memory").await {
        Ok(_) => println!("✅ Successfully deleted old collection"),
        Err(e) => {
            if e.to_string().contains("not found") || e.to_string().contains("Not found") {
                println!("ℹ️  Collection doesn't exist, will create new one");
            } else {
                eprintln!("⚠️  Error deleting collection: {}", e);
            }
        }
    }

    // Create new collection with 1024 dimensions for Voyage AI
    println!("🆕 Creating new collection with 1024 dimensions (Voyage AI)...");
    match client
        .create_collection(
            CreateCollectionBuilder::new("chat_memory")
                .vectors_config(VectorParamsBuilder::new(1024, Distance::Cosine)),
        )
        .await
    {
        Ok(_) => println!("✅ Successfully created collection with 1024 dimensions"),
        Err(e) => {
            eprintln!("❌ Failed to create collection: {}", e);
            return Err(e.into());
        }
    }

    println!("🎉 Qdrant dimension fix complete! Collection now supports Voyage AI (1024 dims)");
    Ok(())
}
