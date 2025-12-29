use rand::RngCore;

fn main() {
    println!("🔐 JWT Secret Key Generator");
    println!("==========================");
    
    // Generate a 256-bit (32-byte) cryptographically secure random key
    let mut key = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut key);
    
    // Encode in different formats
    let base64_key = base64::encode(&key);
    let hex_key = hex::encode(&key);
    
    println!();
    println!("Generated secure JWT secret key:");
    println!("Base64: {}", base64_key);
    println!("Hex:    {}", hex_key);
    println!();
    println!("📝 Copy this line to your .env file:");
    println!("JWT_SECRET={}", base64_key);
    println!();
    println!("✅ This key is cryptographically secure and suitable for production use.");
}