use axum::{
    response::Html,
    routing::get,
    Router,
};

pub fn ui_routes() -> Router {
    Router::new()
        .route("/", get(redirect_to_chat))
        .route("/chat", get(chat_interface))
        .route("/app", get(chat_interface)) // Alternative route
}

pub async fn redirect_to_chat() -> axum::response::Redirect {
    axum::response::Redirect::permanent("/chat")
}

pub async fn chat_interface() -> Html<String> {
    let html = r#"
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>🎬 Agentic Video Editor</title>
    <style>
        * {
            margin: 0;
            padding: 0;
            box-sizing: border-box;
        }

        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, Cantarell, sans-serif;
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            height: 100vh;
            overflow: hidden;
        }

        .app-container {
            display: flex;
            height: 100vh;
            max-width: 1400px;
            margin: 0 auto;
            background: white;
            box-shadow: 0 0 50px rgba(0,0,0,0.1);
        }

        /* Sidebar */
        .sidebar {
            width: 300px;
            background: #2c3e50;
            color: white;
            display: flex;
            flex-direction: column;
            border-right: 1px solid #34495e;
        }

        .sidebar-header {
            padding: 20px;
            background: #1a252f;
            border-bottom: 1px solid #34495e;
        }

        .sidebar-header h1 {
            font-size: 1.5rem;
            margin-bottom: 0.5rem;
        }

        .sidebar-header p {
            color: #bdc3c7;
            font-size: 0.9rem;
        }

        .file-manager {
            flex: 1;
            padding: 20px;
            overflow-y: auto;
        }

        .file-manager h3 {
            margin-bottom: 15px;
            color: #ecf0f1;
            font-size: 1rem;
        }

        .file-list {
            space-y: 8px;
        }

        .file-item {
            background: #34495e;
            padding: 12px;
            border-radius: 8px;
            margin-bottom: 8px;
            cursor: pointer;
            transition: background-color 0.2s;
        }

        .file-item:hover {
            background: #3b4f61;
        }

        .file-name {
            font-weight: 500;
            font-size: 0.9rem;
            margin-bottom: 4px;
        }

        .file-meta {
            font-size: 0.8rem;
            color: #95a5a6;
        }

        .upload-btn {
            margin: 20px;
            padding: 12px;
            background: #3498db;
            color: white;
            border: none;
            border-radius: 8px;
            cursor: pointer;
            font-weight: 500;
            transition: background-color 0.2s;
        }

        .upload-btn:hover {
            background: #2980b9;
        }

        /* Main Content */
        .main-content {
            flex: 1;
            display: flex;
            flex-direction: column;
        }

        .chat-header {
            padding: 20px;
            background: #f8f9fa;
            border-bottom: 1px solid #e9ecef;
            display: flex;
            justify-content: between;
            align-items: center;
        }

        .chat-title {
            font-size: 1.2rem;
            font-weight: 600;
            color: #2c3e50;
        }

        .status-indicator {
            display: flex;
            align-items: center;
            gap: 8px;
            font-size: 0.9rem;
            color: #6c757d;
        }

        .status-dot {
            width: 8px;
            height: 8px;
            border-radius: 50%;
            background: #28a745;
        }

        .status-dot.disconnected {
            background: #dc3545;
        }

        /* Chat Area */
        .chat-container {
            flex: 1;
            display: flex;
            flex-direction: column;
            overflow: hidden;
        }

        .chat-messages {
            flex: 1;
            padding: 20px;
            overflow-y: auto;
            background: #ffffff;
        }

        .message {
            margin-bottom: 20px;
            display: flex;
            align-items: flex-start;
            gap: 12px;
        }

        .message.user {
            flex-direction: row-reverse;
        }

        .message-avatar {
            width: 40px;
            height: 40px;
            border-radius: 50%;
            display: flex;
            align-items: center;
            justify-content: center;
            font-weight: bold;
            color: white;
            font-size: 0.9rem;
        }

        .message.user .message-avatar {
            background: #3498db;
        }

        .message.assistant .message-avatar {
            background: #e74c3c;
        }

        .message-content {
            max-width: 70%;
            padding: 12px 16px;
            border-radius: 18px;
            line-height: 1.4;
        }

        .message.user .message-content {
            background: #3498db;
            color: white;
            border-bottom-right-radius: 4px;
        }

        .message.assistant .message-content {
            background: #f1f3f4;
            color: #2c3e50;
            border-bottom-left-radius: 4px;
        }

        .message-time {
            font-size: 0.8rem;
            color: #6c757d;
            margin-top: 4px;
        }

        /* Input Area */
        .chat-input-container {
            padding: 20px;
            background: #f8f9fa;
            border-top: 1px solid #e9ecef;
        }

        .chat-input-wrapper {
            display: flex;
            gap: 12px;
            align-items: flex-end;
        }

        .chat-input {
            flex: 1;
            min-height: 44px;
            max-height: 120px;
            padding: 12px 16px;
            border: 2px solid #e9ecef;
            border-radius: 22px;
            font-size: 1rem;
            resize: none;
            outline: none;
            transition: border-color 0.2s;
        }

        .chat-input:focus {
            border-color: #3498db;
        }

        .send-btn {
            width: 44px;
            height: 44px;
            border: none;
            border-radius: 50%;
            background: #3498db;
            color: white;
            cursor: pointer;
            display: flex;
            align-items: center;
            justify-content: center;
            transition: background-color 0.2s;
        }

        .send-btn:hover:not(:disabled) {
            background: #2980b9;
        }

        .send-btn:disabled {
            background: #bdc3c7;
            cursor: not-allowed;
        }

        /* Responsive */
        @media (max-width: 768px) {
            .app-container {
                flex-direction: column;
            }
            
            .sidebar {
                width: 100%;
                height: 200px;
            }
            
            .file-manager {
                display: none;
            }
        }

        /* Welcome Screen */
        .welcome-screen {
            display: flex;
            flex-direction: column;
            align-items: center;
            justify-content: center;
            height: 100%;
            text-align: center;
            color: #6c757d;
        }

        .welcome-screen h2 {
            font-size: 1.5rem;
            margin-bottom: 1rem;
            color: #2c3e50;
        }

        .welcome-screen p {
            max-width: 400px;
            line-height: 1.6;
            margin-bottom: 2rem;
        }

        .example-prompts {
            display: flex;
            flex-direction: column;
            gap: 8px;
        }

        .example-prompt {
            padding: 8px 16px;
            background: #f8f9fa;
            border: 1px solid #e9ecef;
            border-radius: 20px;
            cursor: pointer;
            transition: all 0.2s;
            font-size: 0.9rem;
        }

        .example-prompt:hover {
            background: #e9ecef;
            transform: translateY(-1px);
        }

        /* Loading animation */
        .typing-indicator {
            display: none;
            align-items: center;
            gap: 8px;
            padding: 12px 16px;
            background: #f1f3f4;
            border-radius: 18px;
            margin-bottom: 20px;
        }

        .typing-indicator.show {
            display: flex;
        }

        .typing-dots {
            display: flex;
            gap: 4px;
        }

        .typing-dot {
            width: 8px;
            height: 8px;
            border-radius: 50%;
            background: #6c757d;
            animation: typing 1.4s infinite;
        }

        .typing-dot:nth-child(2) {
            animation-delay: 0.2s;
        }

        .typing-dot:nth-child(3) {
            animation-delay: 0.4s;
        }

        @keyframes typing {
            0%, 60%, 100% {
                transform: translateY(0);
                opacity: 0.4;
            }
            30% {
                transform: translateY(-10px);
                opacity: 1;
            }
        }
    </style>
</head>
<body>
    <div class="app-container">
        <!-- Sidebar -->
        <div class="sidebar">
            <div class="sidebar-header">
                <h1>🎬 Video Editor</h1>
                <p>AI-powered video editing</p>
            </div>
            
            <div class="file-manager">
                <h3>📁 Uploaded Files</h3>
                <div id="fileList" class="file-list">
                    <div class="file-item" style="opacity: 0.5;">
                        <div class="file-name">No files uploaded yet</div>
                        <div class="file-meta">Upload files to get started</div>
                    </div>
                </div>
            </div>
            
            <button class="upload-btn" onclick="uploadFiles()">
                📤 Upload Files
            </button>
        </div>

        <!-- Main Content -->
        <div class="main-content">
            <div class="chat-header">
                <div class="chat-title">Video Editing Assistant</div>
                <div class="status-indicator">
                    <div id="statusDot" class="status-dot disconnected"></div>
                    <span id="statusText">Connecting...</span>
                </div>
            </div>

            <div class="chat-container">
                <div id="chatMessages" class="chat-messages">
                    <div class="welcome-screen">
                        <h2>Welcome to your AI Video Editor! 🎬</h2>
                        <p>I can help you edit videos using natural language. Upload your files and tell me what you'd like to do!</p>
                        
                        <div class="example-prompts">
                            <div class="example-prompt" onclick="sendExamplePrompt('Trim my video from 10 seconds to 30 seconds')">
                                "Trim my video from 10 seconds to 30 seconds"
                            </div>
                            <div class="example-prompt" onclick="sendExamplePrompt('Add text overlay saying Hello World')">
                                "Add text overlay saying 'Hello World'"
                            </div>
                            <div class="example-prompt" onclick="sendExamplePrompt('Convert my video to MP4 format')">
                                "Convert my video to MP4 format"
                            </div>
                            <div class="example-prompt" onclick="sendExamplePrompt('Analyze my video and tell me its properties')">
                                "Analyze my video and tell me its properties"
                            </div>
                        </div>
                    </div>
                </div>

                <div class="typing-indicator" id="typingIndicator">
                    <div class="message-avatar" style="background: #e74c3c;">🤖</div>
                    <div style="display: flex; align-items: center; gap: 8px;">
                        <span>AI is thinking</span>
                        <div class="typing-dots">
                            <div class="typing-dot"></div>
                            <div class="typing-dot"></div>
                            <div class="typing-dot"></div>
                        </div>
                    </div>
                </div>

                <div class="chat-input-container">
                    <div class="chat-input-wrapper">
                        <textarea 
                            id="chatInput" 
                            class="chat-input" 
                            placeholder="Ask me to edit your videos... (e.g., 'trim my video from 10s to 30s')"
                            rows="1"
                        ></textarea>
                        <button id="sendBtn" class="send-btn" onclick="sendMessage()">
                            <svg width="20" height="20" viewBox="0 0 24 24" fill="currentColor">
                                <path d="M2.01 21L23 12 2.01 3 2 10l15 2-15 2z"/>
                            </svg>
                        </button>
                    </div>
                </div>
            </div>
        </div>
    </div>

    <!-- Hidden file input -->
    <input type="file" id="fileInput" multiple accept="video/*,audio/*,image/*,.pdf,.doc,.docx,.txt" style="display: none;">

    <script>
        let ws = null;
        let isConnected = false;
        let uploadedFiles = [];

        // Initialize the application
        document.addEventListener('DOMContentLoaded', function() {
            initializeWebSocket();
            setupEventListeners();
            loadUploadedFiles();
        });

        function initializeWebSocket() {
            const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
            const wsUrl = `${protocol}//${window.location.host}/ws`;
            
            ws = new WebSocket(wsUrl);
            
            ws.onopen = function() {
                isConnected = true;
                updateConnectionStatus(true);
                console.log('Connected to video editing assistant');
            };
            
            ws.onmessage = function(event) {
                hideTypingIndicator();
                addMessage('assistant', event.data);
            };
            
            ws.onclose = function() {
                isConnected = false;
                updateConnectionStatus(false);
                console.log('Disconnected from assistant');
                
                // Try to reconnect after 3 seconds
                setTimeout(initializeWebSocket, 3000);
            };
            
            ws.onerror = function(error) {
                console.error('WebSocket error:', error);
                hideTypingIndicator();
                updateConnectionStatus(false);
            };
        }

        function setupEventListeners() {
            const chatInput = document.getElementById('chatInput');
            const sendBtn = document.getElementById('sendBtn');
            
            // Auto-resize textarea
            chatInput.addEventListener('input', function() {
                this.style.height = 'auto';
                this.style.height = Math.min(this.scrollHeight, 120) + 'px';
            });
            
            // Send on Enter, new line on Shift+Enter
            chatInput.addEventListener('keydown', function(e) {
                if (e.key === 'Enter' && !e.shiftKey) {
                    e.preventDefault();
                    sendMessage();
                }
            });
            
            // File input handler
            document.getElementById('fileInput').addEventListener('change', handleFileUpload);
        }

        function updateConnectionStatus(connected) {
            const statusDot = document.getElementById('statusDot');
            const statusText = document.getElementById('statusText');
            
            if (connected) {
                statusDot.classList.remove('disconnected');
                statusText.textContent = 'Connected';
            } else {
                statusDot.classList.add('disconnected');
                statusText.textContent = 'Disconnected';
            }
        }

        function sendMessage() {
            const input = document.getElementById('chatInput');
            const message = input.value.trim();
            
            if (!message || !isConnected) return;
            
            // Add user message to chat
            addMessage('user', message);
            
            // Clear input
            input.value = '';
            input.style.height = 'auto';
            
            // Show typing indicator
            showTypingIndicator();
            
            // Send to WebSocket
            ws.send(message);
        }

        function sendExamplePrompt(prompt) {
            document.getElementById('chatInput').value = prompt;
            sendMessage();
        }

        function addMessage(sender, content) {
            const messagesContainer = document.getElementById('chatMessages');
            const welcomeScreen = messagesContainer.querySelector('.welcome-screen');
            
            // Hide welcome screen on first message
            if (welcomeScreen) {
                welcomeScreen.style.display = 'none';
            }
            
            const messageDiv = document.createElement('div');
            messageDiv.className = `message ${sender}`;
            
            const now = new Date();
            const timeString = now.toLocaleTimeString([], {hour: '2-digit', minute:'2-digit'});
            
            messageDiv.innerHTML = `
                <div class="message-avatar">
                    ${sender === 'user' ? '👤' : '🤖'}
                </div>
                <div class="message-content">
                    ${content}
                    <div class="message-time">${timeString}</div>
                </div>
            `;
            
            messagesContainer.appendChild(messageDiv);
            messagesContainer.scrollTop = messagesContainer.scrollHeight;
        }

        function showTypingIndicator() {
            document.getElementById('typingIndicator').classList.add('show');
            const messagesContainer = document.getElementById('chatMessages');
            messagesContainer.scrollTop = messagesContainer.scrollHeight;
        }

        function hideTypingIndicator() {
            document.getElementById('typingIndicator').classList.remove('show');
        }

        function uploadFiles() {
            document.getElementById('fileInput').click();
        }

        async function handleFileUpload(event) {
            const files = event.target.files;
            if (files.length === 0) return;
            
            const formData = new FormData();
            for (let file of files) {
                formData.append('files', file);
            }
            
            try {
                const response = await fetch('/upload', {
                    method: 'POST',
                    body: formData
                });
                
                const result = await response.json();
                
                if (result.success) {
                    uploadedFiles = [...uploadedFiles, ...result.files];
                    updateFileList();
                    addMessage('assistant', `✅ Successfully uploaded ${result.files.length} files: ${result.files.map(f => f.original_name).join(', ')}`);
                } else {
                    addMessage('assistant', '❌ Failed to upload files. Please try again.');
                }
            } catch (error) {
                console.error('Upload error:', error);
                addMessage('assistant', '❌ Error uploading files: ' + error.message);
            }
            
            // Clear the file input
            event.target.value = '';
        }

        function updateFileList() {
            const fileList = document.getElementById('fileList');
            
            if (uploadedFiles.length === 0) {
                fileList.innerHTML = `
                    <div class="file-item" style="opacity: 0.5;">
                        <div class="file-name">No files uploaded yet</div>
                        <div class="file-meta">Upload files to get started</div>
                    </div>
                `;
                return;
            }
            
            fileList.innerHTML = uploadedFiles.map(file => `
                <div class="file-item" onclick="selectFile('${file.id}')">
                    <div class="file-name">${file.original_name}</div>
                    <div class="file-meta">${(file.size / 1024 / 1024).toFixed(2)} MB • ${file.file_type}</div>
                </div>
            `).join('');
        }

        function selectFile(fileId) {
            const file = uploadedFiles.find(f => f.id === fileId);
            if (file) {
                const input = document.getElementById('chatInput');
                input.value = `Please work with my file: ${file.original_name}`;
                input.focus();
            }
        }

        function loadUploadedFiles() {
            // TODO: Load previously uploaded files from server/localStorage
            // For now, we'll just show empty state
        }
    </script>
</body>
</html>
    "#;
    
    Html(html.to_string())
}