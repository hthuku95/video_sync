"""
Agentic Video Editor API - Python Client Example

This is a complete Python client application that demonstrates how to use the 
Agentic Video Editor API with JWT authentication and various endpoints.

Requirements:
    pip install requests

Usage:
    python api_client_demo.py

Features:
- User registration and login
- JWT token management
- File upload functionality
- Admin statistics and user management
- Email whitelist management
- Comprehensive error handling
"""

import requests
import json
from typing import Optional, Dict, Any
from datetime import datetime
import os

class VideoEditorAPIClient:
    """Python client for the Agentic Video Editor API"""
    
    def __init__(self, base_url: str = "http://localhost:3000"):
        self.base_url = base_url.rstrip('/')
        self.token: Optional[str] = None
        self.user_info: Optional[Dict] = None
        
    def _get_headers(self, include_auth: bool = True) -> Dict[str, str]:
        """Get headers for API requests"""
        headers = {"Content-Type": "application/json"}
        if include_auth and self.token:
            headers["Authorization"] = f"Bearer {self.token}"
        return headers
    
    def register(self, email: str, username: str, password: str) -> Dict[str, Any]:
        """Register a new user account"""
        url = f"{self.base_url}/api/auth/register"
        data = {
            "email": email,
            "username": username,
            "password": password
        }
        
        response = requests.post(url, json=data, headers=self._get_headers(include_auth=False))
        result = response.json()
        
        if result.get("success") and "token" in result:
            self.token = result["token"]
            self.user_info = result["user"]
            print(f"✅ Registered successfully as {username}")
        else:
            print(f"❌ Registration failed: {result.get('message', 'Unknown error')}")
            
        return result
    
    def login(self, email: str, password: str) -> Dict[str, Any]:
        """Login and get JWT token"""
        url = f"{self.base_url}/api/auth/login"
        data = {"email": email, "password": password}
        
        response = requests.post(url, json=data, headers=self._get_headers(include_auth=False))
        result = response.json()
        
        if result.get("success") and "token" in result:
            self.token = result["token"]
            self.user_info = result["user"]
            print(f"✅ Logged in as {self.user_info['username']}")
            print(f"   Role: {'Superuser' if self.user_info['is_superuser'] else 'Staff' if self.user_info['is_staff'] else 'User'}")
        else:
            print(f"❌ Login failed: {result.get('message', 'Unknown error')}")
            
        return result
    
    def verify_token(self) -> Dict[str, Any]:
        """Verify current JWT token"""
        if not self.token:
            return {"success": False, "message": "No token available"}
            
        url = f"{self.base_url}/api/auth/verify"
        response = requests.get(url, headers=self._get_headers())
        result = response.json()
        
        if result.get("success"):
            print("✅ Token is valid")
        else:
            print(f"❌ Token verification failed: {result.get('message', 'Invalid token')}")
            self.token = None
            self.user_info = None
            
        return result
    
    def upload_file(self, file_path: str, session_id: Optional[str] = None) -> Dict[str, Any]:
        """Upload a file to the API"""
        if session_id:
            url = f"{self.base_url}/upload/session/{session_id}"
        else:
            url = f"{self.base_url}/upload"
        
        # For file upload, don't use JSON content-type
        headers = {}
        if self.token:
            headers["Authorization"] = f"Bearer {self.token}"
        
        try:
            with open(file_path, 'rb') as f:
                files = {'files': (os.path.basename(file_path), f)}
                response = requests.post(url, files=files, headers=headers)
                result = response.json()
                
                if result.get("success"):
                    print(f"✅ File uploaded successfully: {os.path.basename(file_path)}")
                    if "files" in result:
                        for file_info in result["files"]:
                            print(f"   File ID: {file_info.get('id')}")
                            print(f"   Size: {file_info.get('size')} bytes")
                else:
                    print(f"❌ File upload failed: {result.get('message', 'Unknown error')}")
                    
                return result
        except FileNotFoundError:
            print(f"❌ File not found: {file_path}")
            return {"success": False, "message": "File not found"}
        except Exception as e:
            print(f"❌ Upload error: {str(e)}")
            return {"success": False, "message": str(e)}
    
    def get_api_status(self) -> Dict[str, Any]:
        """Get API health status"""
        url = f"{self.base_url}/api/status"
        response = requests.get(url)
        result = response.json()
        
        print("📊 API Status:")
        print(f"   Status: {result.get('status', 'Unknown')}")
        print(f"   Version: {result.get('version', 'Unknown')}")
        print(f"   Timestamp: {result.get('timestamp', 'Unknown')}")
        
        return result
    
    # Admin Functions (require staff/superuser privileges)
    def get_admin_stats(self) -> Dict[str, Any]:
        """Get admin statistics"""
        if not self.token:
            print("❌ Authentication required")
            return {"success": False, "message": "Not authenticated"}
            
        url = f"{self.base_url}/api/admin/stats"
        response = requests.get(url, headers=self._get_headers())
        result = response.json()
        
        if result.get("success"):
            stats = result["stats"]
            print("📈 Admin Statistics:")
            print(f"   Total Users: {stats['total_users']}")
            print(f"   Active Users: {stats['active_users']}")
            print(f"   Chat Sessions: {stats['total_chat_sessions']}")
            print(f"   Uploaded Files: {stats['total_files']}")
        else:
            print(f"❌ Failed to get admin stats: {result.get('message', 'Access denied')}")
            
        return result
    
    def get_whitelist_status(self) -> Dict[str, Any]:
        """Get email whitelist status"""
        if not self.token:
            print("❌ Authentication required")
            return {"success": False, "message": "Not authenticated"}
            
        url = f"{self.base_url}/api/admin/whitelist/status"
        response = requests.get(url, headers=self._get_headers())
        result = response.json()
        
        if result.get("success"):
            status = result["status"]
            print("🛡️ Whitelist Status:")
            print(f"   Enabled: {'✅ YES' if status['enabled'] else '❌ NO'}")
            print(f"   Total Whitelisted Emails: {status['total_emails']}")
        else:
            print(f"❌ Failed to get whitelist status: {result.get('message', 'Access denied')}")
            
        return result
    
    def toggle_whitelist(self, enabled: bool) -> Dict[str, Any]:
        """Enable or disable email whitelist"""
        if not self.token:
            print("❌ Authentication required")
            return {"success": False, "message": "Not authenticated"}
            
        url = f"{self.base_url}/api/admin/whitelist/toggle"
        data = {"enabled": enabled}
        response = requests.post(url, json=data, headers=self._get_headers())
        result = response.json()
        
        if result.get("success"):
            action = "enabled" if enabled else "disabled"
            print(f"✅ Whitelist {action} successfully")
        else:
            print(f"❌ Failed to toggle whitelist: {result.get('message', 'Access denied')}")
            
        return result
    
    def add_email_to_whitelist(self, email: str) -> Dict[str, Any]:
        """Add an email to the whitelist"""
        if not self.token:
            print("❌ Authentication required")
            return {"success": False, "message": "Not authenticated"}
            
        url = f"{self.base_url}/api/admin/whitelist/emails"
        data = {"email": email}
        response = requests.post(url, json=data, headers=self._get_headers())
        result = response.json()
        
        if result.get("success"):
            print(f"✅ Added {email} to whitelist")
        else:
            print(f"❌ Failed to add email: {result.get('message', 'Unknown error')}")
            
        return result
    
    def get_whitelisted_emails(self) -> Dict[str, Any]:
        """Get list of whitelisted emails"""
        if not self.token:
            print("❌ Authentication required")
            return {"success": False, "message": "Not authenticated"}
            
        url = f"{self.base_url}/api/admin/whitelist/emails"
        response = requests.get(url, headers=self._get_headers())
        result = response.json()
        
        if result.get("success"):
            emails = result["emails"]
            print(f"📧 Whitelisted Emails ({len(emails)} total):")
            for email_info in emails:
                created_date = datetime.fromisoformat(email_info['created_at'].replace('Z', '+00:00'))
                print(f"   • {email_info['email']} (added {created_date.strftime('%Y-%m-%d')})")
        else:
            print(f"❌ Failed to get whitelisted emails: {result.get('message', 'Access denied')}")
            
        return result
    
    def remove_email_from_whitelist(self, email_id: int) -> Dict[str, Any]:
        """Remove an email from the whitelist by ID"""
        if not self.token:
            print("❌ Authentication required")
            return {"success": False, "message": "Not authenticated"}
            
        url = f"{self.base_url}/api/admin/whitelist/emails/{email_id}"
        response = requests.delete(url, headers=self._get_headers())
        result = response.json()
        
        if result.get("success"):
            print(f"✅ Removed email from whitelist")
        else:
            print(f"❌ Failed to remove email: {result.get('message', 'Unknown error')}")
            
        return result
    
    def get_users_list(self, page: int = 1, limit: int = 20, search: str = "") -> Dict[str, Any]:
        """Get list of users with pagination"""
        if not self.token:
            print("❌ Authentication required")
            return {"success": False, "message": "Not authenticated"}
            
        url = f"{self.base_url}/api/admin/users"
        params = {"page": page, "limit": limit}
        if search:
            params["search"] = search
            
        response = requests.get(url, params=params, headers=self._get_headers())
        result = response.json()
        
        if result.get("success"):
            users = result["users"]
            pagination = result["pagination"]
            print(f"👥 Users (Page {pagination['page']} of {pagination['total_pages']}):")
            print(f"   Total Users: {pagination['total']}")
            
            for user in users:
                role = "Superuser" if user['is_superuser'] else "Staff" if user['is_staff'] else "User"
                status = "Active" if user['is_active'] else "Inactive"
                created_date = datetime.fromisoformat(user['created_at'].replace('Z', '+00:00'))
                print(f"   • {user['username']} ({user['email']}) - {role} - {status} - {created_date.strftime('%Y-%m-%d')}")
        else:
            print(f"❌ Failed to get users: {result.get('message', 'Access denied')}")
            
        return result


def demo_basic_functionality(client: VideoEditorAPIClient):
    """Demo basic API functionality"""
    print("🎬 Agentic Video Editor API Client Demo")
    print("=" * 50)
    
    # Check API status (public endpoint)
    print("\n1. Checking API Status...")
    client.get_api_status()
    
    # Try to register a new user
    print("\n2. Registering new user...")
    register_result = client.register(
        email="demo@example.com",
        username="demouser",
        password="password123"
    )
    
    # If registration failed (user exists), try login instead
    if not register_result.get("success"):
        print("\n2b. Trying to login instead...")
        client.login("demo@example.com", "password123")
    
    # Verify token
    print("\n3. Verifying authentication...")
    client.verify_token()
    
    return client.token is not None


def demo_file_upload(client: VideoEditorAPIClient):
    """Demo file upload functionality"""
    print("\n4. Testing file upload...")
    
    # Create a dummy test file for demonstration
    test_file = "test_demo.txt"
    try:
        with open(test_file, 'w') as f:
            f.write("This is a test file for the API demo.\nCreated for upload testing purposes.")
        
        client.upload_file(test_file)
        
        # Clean up test file
        os.remove(test_file)
    except Exception as e:
        print(f"   (File upload demo skipped - {e})")


def demo_admin_functionality(client: VideoEditorAPIClient):
    """Demo admin functionality"""
    print("\n5. Testing admin functions...")
    
    # Get admin stats
    print("\n5a. Getting admin statistics...")
    stats_result = client.get_admin_stats()
    
    # If user doesn't have admin privileges, stop here
    if not stats_result.get("success"):
        print("   (Admin functions require staff/superuser privileges)")
        return
    
    # Get users list
    print("\n5b. Getting users list...")
    client.get_users_list(limit=5)
    
    # Get whitelist status
    print("\n5c. Checking whitelist status...")
    client.get_whitelist_status()
    
    # Get whitelisted emails
    print("\n5d. Getting whitelisted emails...")
    client.get_whitelisted_emails()
    
    # Example: Add an email to whitelist (uncomment to test)
    print("\n5e. Example: Adding email to whitelist...")
    print("   (Uncomment the line below to test adding an email)")
    # client.add_email_to_whitelist("newuser@example.com")
    
    # Example: Toggle whitelist (uncomment to test)
    print("\n5f. Example: Toggling whitelist...")
    print("   (Uncomment the lines below to test toggling)")
    # client.toggle_whitelist(True)   # Enable whitelist
    # client.toggle_whitelist(False)  # Disable whitelist


def interactive_demo():
    """Interactive demo that lets user try different functions"""
    client = VideoEditorAPIClient("http://localhost:3000")
    
    print("🎬 Interactive Agentic Video Editor API Demo")
    print("=" * 50)
    
    while True:
        print("\nAvailable actions:")
        print("1. Check API Status")
        print("2. Register User")
        print("3. Login")
        print("4. Verify Token")
        print("5. Get Admin Stats")
        print("6. Get Users List")
        print("7. Get Whitelist Status")
        print("8. Get Whitelisted Emails")
        print("9. Add Email to Whitelist")
        print("10. Toggle Whitelist")
        print("0. Exit")
        
        try:
            choice = input("\nEnter your choice (0-10): ").strip()
            
            if choice == "0":
                print("👋 Goodbye!")
                break
            elif choice == "1":
                client.get_api_status()
            elif choice == "2":
                email = input("Enter email: ")
                username = input("Enter username: ")
                password = input("Enter password: ")
                client.register(email, username, password)
            elif choice == "3":
                email = input("Enter email: ")
                password = input("Enter password: ")
                client.login(email, password)
            elif choice == "4":
                client.verify_token()
            elif choice == "5":
                client.get_admin_stats()
            elif choice == "6":
                client.get_users_list()
            elif choice == "7":
                client.get_whitelist_status()
            elif choice == "8":
                client.get_whitelisted_emails()
            elif choice == "9":
                email = input("Enter email to whitelist: ")
                client.add_email_to_whitelist(email)
            elif choice == "10":
                enabled = input("Enable whitelist? (y/n): ").lower().startswith('y')
                client.toggle_whitelist(enabled)
            else:
                print("❌ Invalid choice")
                
        except KeyboardInterrupt:
            print("\n👋 Goodbye!")
            break
        except Exception as e:
            print(f"❌ Error: {e}")


def main():
    """Main function - choose demo mode"""
    print("Choose demo mode:")
    print("1. Automatic demo (recommended)")
    print("2. Interactive demo")
    
    try:
        choice = input("Enter choice (1 or 2): ").strip()
        
        if choice == "2":
            interactive_demo()
        else:
            # Run automatic demo
            client = VideoEditorAPIClient("http://localhost:3000")
            
            # Run basic functionality demo
            if demo_basic_functionality(client):
                # Only run other demos if authentication succeeded
                demo_file_upload(client)
                demo_admin_functionality(client)
            
            print("\n✅ Demo completed!")
            
    except KeyboardInterrupt:
        print("\n👋 Goodbye!")


if __name__ == "__main__":
    # Install required packages: pip install requests
    main()