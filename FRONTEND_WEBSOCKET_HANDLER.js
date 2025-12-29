// WebSocket Handler for Video Editor Frontend
// Handles progress updates, chat messages, and typing indicators
// Updated: 2025-12-12

class VideoEditorWebSocket {
  constructor(sessionId, model = 'gemini') {
    this.sessionId = sessionId;
    this.model = model;
    this.socket = null;
    this.reconnectAttempts = 0;
    this.maxReconnectAttempts = 5;
    this.reconnectDelay = 2000;

    // Callbacks
    this.onMessage = null;
    this.onProgress = null;
    this.onError = null;
    this.onConnected = null;
    this.onDisconnected = null;

    this.connect();
  }

  connect() {
    const wsUrl = `ws://localhost:3000/ws?session=${this.sessionId}&model=${this.model}`;
    console.log('🔌 Connecting to WebSocket:', wsUrl);

    this.socket = new WebSocket(wsUrl);

    this.socket.onopen = () => {
      console.log('✅ WebSocket connected');
      this.reconnectAttempts = 0;
      if (this.onConnected) this.onConnected();
    };

    this.socket.onmessage = (event) => {
      try {
        const data = JSON.parse(event.data);
        this.handleMessage(data);
      } catch (error) {
        console.error('❌ Failed to parse WebSocket message:', error);
      }
    };

    this.socket.onerror = (error) => {
      console.error('❌ WebSocket error:', error);
      if (this.onError) this.onError(error);
    };

    this.socket.onclose = () => {
      console.log('🔌 WebSocket disconnected');
      if (this.onDisconnected) this.onDisconnected();
      this.attemptReconnect();
    };
  }

  handleMessage(data) {
    console.log('📨 Received:', data);

    switch (data.type) {
      case 'progress':
        // ⚠️ DO NOT add to chat history
        // Show in typing indicator or progress popup
        if (this.onProgress) {
          this.onProgress(data.content, data.timestamp);
        }
        break;

      case 'message':
        // ✅ Add to chat history
        if (this.onMessage) {
          this.onMessage({
            role: 'assistant',
            content: data.content,
            timestamp: data.timestamp
          });
        }
        break;

      default:
        console.warn('⚠️ Unknown message type:', data.type, data);
    }
  }

  sendMessage(content) {
    if (this.socket && this.socket.readyState === WebSocket.OPEN) {
      const message = JSON.stringify({
        type: 'user_message',
        content: content
      });
      this.socket.send(message);
      return true;
    } else {
      console.error('❌ WebSocket not connected');
      return false;
    }
  }

  attemptReconnect() {
    if (this.reconnectAttempts < this.maxReconnectAttempts) {
      this.reconnectAttempts++;
      console.log(`🔄 Reconnecting in ${this.reconnectDelay}ms (attempt ${this.reconnectAttempts}/${this.maxReconnectAttempts})`);

      setTimeout(() => {
        this.connect();
      }, this.reconnectDelay);

      this.reconnectDelay *= 2; // Exponential backoff
    } else {
      console.error('❌ Max reconnection attempts reached');
    }
  }

  disconnect() {
    if (this.socket) {
      this.socket.close();
      this.socket = null;
    }
  }
}

// ============================================================================
// USAGE EXAMPLE 1: Vanilla JavaScript
// ============================================================================

function initializeChat() {
  const sessionId = generateSessionId(); // Your session ID generation
  const ws = new VideoEditorWebSocket(sessionId, 'gemini');

  // Handle AI messages (add to chat)
  ws.onMessage = (message) => {
    addMessageToChat('assistant', message.content, message.timestamp);
    hideTypingIndicator();
  };

  // Handle progress updates (show in typing indicator)
  ws.onProgress = (content, timestamp) => {
    showTypingIndicator(content);
  };

  // Handle errors
  ws.onError = (error) => {
    console.error('WebSocket error:', error);
    showErrorToast('Connection error. Please refresh.');
  };

  // Handle connection status
  ws.onConnected = () => {
    console.log('Connected to AI agent');
    updateConnectionStatus('connected');
  };

  ws.onDisconnected = () => {
    console.log('Disconnected from AI agent');
    updateConnectionStatus('disconnected');
  };

  // Send message when user submits
  document.querySelector('#send-button').addEventListener('click', () => {
    const input = document.querySelector('#message-input');
    const message = input.value.trim();

    if (message) {
      // Add user message to chat immediately
      addMessageToChat('user', message, new Date().toISOString());

      // Send to backend
      ws.sendMessage(message);

      // Clear input
      input.value = '';
    }
  });

  return ws;
}

// Helper: Add message to chat
function addMessageToChat(role, content, timestamp) {
  const chatContainer = document.querySelector('.chat-messages');
  const messageDiv = document.createElement('div');
  messageDiv.className = `message ${role}`;

  messageDiv.innerHTML = `
    <div class="message-avatar">
      ${role === 'user' ? '👤' : '🤖'}
    </div>
    <div class="message-content">
      ${formatMarkdown(content)}
    </div>
    <div class="message-time">
      ${formatTime(timestamp)}
    </div>
  `;

  chatContainer.appendChild(messageDiv);
  chatContainer.scrollTop = chatContainer.scrollHeight;
}

// Helper: Show typing indicator
function showTypingIndicator(message) {
  const indicator = document.querySelector('.typing-indicator');
  if (indicator) {
    indicator.querySelector('.message').textContent = message;
    indicator.style.display = 'flex';
  }
}

// Helper: Hide typing indicator
function hideTypingIndicator() {
  const indicator = document.querySelector('.typing-indicator');
  if (indicator) {
    indicator.style.display = 'none';
  }
}

// Helper: Format markdown
function formatMarkdown(content) {
  return content
    .replace(/\*\*(.*?)\*\*/g, '<strong>$1</strong>')
    .replace(/\*(.*?)\*/g, '<em>$1</em>')
    .replace(/\n/g, '<br>');
}

// Helper: Format timestamp
function formatTime(timestamp) {
  const date = new Date(timestamp);
  return date.toLocaleTimeString('en-US', {
    hour: '2-digit',
    minute: '2-digit'
  });
}

// Helper: Generate session ID
function generateSessionId() {
  return 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, function(c) {
    const r = Math.random() * 16 | 0;
    const v = c === 'x' ? r : (r & 0x3 | 0x8);
    return v.toString(16);
  });
}

// ============================================================================
// USAGE EXAMPLE 2: React Component
// ============================================================================

/**
 * React Hook for WebSocket Chat
 */
function useVideoEditorChat(sessionId, model = 'gemini') {
  const [messages, setMessages] = React.useState([]);
  const [progress, setProgress] = React.useState(null);
  const [isConnected, setIsConnected] = React.useState(false);
  const wsRef = React.useRef(null);

  React.useEffect(() => {
    const ws = new VideoEditorWebSocket(sessionId, model);

    ws.onMessage = (message) => {
      setMessages(prev => [...prev, message]);
      setProgress(null); // Hide progress when message arrives
    };

    ws.onProgress = (content) => {
      setProgress(content);
    };

    ws.onConnected = () => {
      setIsConnected(true);
    };

    ws.onDisconnected = () => {
      setIsConnected(false);
      setProgress(null);
    };

    wsRef.current = ws;

    return () => {
      ws.disconnect();
    };
  }, [sessionId, model]);

  const sendMessage = (content) => {
    if (wsRef.current) {
      // Add user message to state
      setMessages(prev => [...prev, {
        role: 'user',
        content,
        timestamp: new Date().toISOString()
      }]);

      // Send to backend
      wsRef.current.sendMessage(content);
    }
  };

  return {
    messages,
    progress,
    isConnected,
    sendMessage
  };
}

/**
 * React Chat Component Example
 */
function ChatComponent({ sessionId }) {
  const { messages, progress, isConnected, sendMessage } = useVideoEditorChat(sessionId);
  const [input, setInput] = React.useState('');

  const handleSubmit = (e) => {
    e.preventDefault();
    if (input.trim()) {
      sendMessage(input.trim());
      setInput('');
    }
  };

  return (
    <div className="chat-container">
      {/* Connection Status */}
      <div className={`connection-status ${isConnected ? 'connected' : 'disconnected'}`}>
        {isConnected ? '🟢 Connected' : '🔴 Disconnected'}
      </div>

      {/* Messages */}
      <div className="chat-messages">
        {messages.map((msg, index) => (
          <div key={index} className={`message ${msg.role}`}>
            <div className="message-avatar">
              {msg.role === 'user' ? '👤' : '🤖'}
            </div>
            <div className="message-content">
              {msg.content}
            </div>
            <div className="message-time">
              {new Date(msg.timestamp).toLocaleTimeString()}
            </div>
          </div>
        ))}

        {/* Typing Indicator */}
        {progress && (
          <div className="typing-indicator">
            <div className="spinner"></div>
            <div className="message">{progress}</div>
          </div>
        )}
      </div>

      {/* Input */}
      <form onSubmit={handleSubmit} className="chat-input">
        <input
          type="text"
          value={input}
          onChange={(e) => setInput(e.target.value)}
          placeholder="Type your message..."
          disabled={!isConnected}
        />
        <button type="submit" disabled={!isConnected || !input.trim()}>
          Send
        </button>
      </form>
    </div>
  );
}

// ============================================================================
// USAGE EXAMPLE 3: Vue.js Component
// ============================================================================

/**
 * Vue 3 Composition API Example
 */
const ChatComponent = {
  setup() {
    const messages = Vue.ref([]);
    const progress = Vue.ref(null);
    const isConnected = Vue.ref(false);
    const input = Vue.ref('');
    const ws = Vue.ref(null);

    const sessionId = generateSessionId();

    Vue.onMounted(() => {
      ws.value = new VideoEditorWebSocket(sessionId, 'gemini');

      ws.value.onMessage = (message) => {
        messages.value.push(message);
        progress.value = null;
      };

      ws.value.onProgress = (content) => {
        progress.value = content;
      };

      ws.value.onConnected = () => {
        isConnected.value = true;
      };

      ws.value.onDisconnected = () => {
        isConnected.value = false;
        progress.value = null;
      };
    });

    Vue.onUnmounted(() => {
      if (ws.value) {
        ws.value.disconnect();
      }
    });

    const sendMessage = () => {
      if (input.value.trim() && ws.value) {
        messages.value.push({
          role: 'user',
          content: input.value.trim(),
          timestamp: new Date().toISOString()
        });

        ws.value.sendMessage(input.value.trim());
        input.value = '';
      }
    };

    return {
      messages,
      progress,
      isConnected,
      input,
      sendMessage
    };
  },

  template: `
    <div class="chat-container">
      <div :class="['connection-status', isConnected ? 'connected' : 'disconnected']">
        {{ isConnected ? '🟢 Connected' : '🔴 Disconnected' }}
      </div>

      <div class="chat-messages">
        <div v-for="(msg, index) in messages" :key="index" :class="['message', msg.role]">
          <div class="message-avatar">{{ msg.role === 'user' ? '👤' : '🤖' }}</div>
          <div class="message-content">{{ msg.content }}</div>
          <div class="message-time">{{ new Date(msg.timestamp).toLocaleTimeString() }}</div>
        </div>

        <div v-if="progress" class="typing-indicator">
          <div class="spinner"></div>
          <div class="message">{{ progress }}</div>
        </div>
      </div>

      <div class="chat-input">
        <input
          v-model="input"
          @keypress.enter="sendMessage"
          placeholder="Type your message..."
          :disabled="!isConnected"
        />
        <button @click="sendMessage" :disabled="!isConnected || !input.trim()">
          Send
        </button>
      </div>
    </div>
  `
};

// ============================================================================
// CSS STYLES (Add to your stylesheet)
// ============================================================================

const styles = `
/* Chat Container */
.chat-container {
  display: flex;
  flex-direction: column;
  height: 100vh;
  max-width: 800px;
  margin: 0 auto;
  background: #fff;
}

/* Connection Status */
.connection-status {
  padding: 8px 16px;
  text-align: center;
  font-size: 12px;
  font-weight: 600;
}

.connection-status.connected {
  background: #d4edda;
  color: #155724;
}

.connection-status.disconnected {
  background: #f8d7da;
  color: #721c24;
}

/* Messages Container */
.chat-messages {
  flex: 1;
  overflow-y: auto;
  padding: 20px;
  background: #f5f5f5;
}

/* Individual Message */
.message {
  display: flex;
  gap: 12px;
  margin-bottom: 16px;
  animation: slideIn 0.3s ease;
}

.message.user {
  flex-direction: row-reverse;
}

.message-avatar {
  width: 36px;
  height: 36px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 20px;
  flex-shrink: 0;
  background: #e0e0e0;
}

.message.user .message-avatar {
  background: #667eea;
}

.message-content {
  max-width: 70%;
  padding: 12px 16px;
  border-radius: 12px;
  background: #fff;
  box-shadow: 0 1px 2px rgba(0,0,0,0.1);
}

.message.user .message-content {
  background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
  color: white;
}

.message-time {
  font-size: 11px;
  color: #999;
  margin-top: 4px;
}

/* Typing Indicator */
.typing-indicator {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px 16px;
  background: rgba(102, 126, 234, 0.1);
  border-radius: 12px;
  margin-bottom: 16px;
  animation: pulse 2s ease-in-out infinite;
}

.typing-indicator .spinner {
  width: 20px;
  height: 20px;
  border: 3px solid rgba(102, 126, 234, 0.3);
  border-top-color: #667eea;
  border-radius: 50%;
  animation: spin 1s linear infinite;
}

.typing-indicator .message {
  color: #667eea;
  font-style: italic;
  font-size: 14px;
}

/* Chat Input */
.chat-input {
  display: flex;
  gap: 12px;
  padding: 16px;
  background: #fff;
  border-top: 1px solid #e0e0e0;
}

.chat-input input {
  flex: 1;
  padding: 12px 16px;
  border: 1px solid #e0e0e0;
  border-radius: 24px;
  font-size: 14px;
  outline: none;
}

.chat-input input:focus {
  border-color: #667eea;
  box-shadow: 0 0 0 3px rgba(102, 126, 234, 0.1);
}

.chat-input button {
  padding: 12px 24px;
  background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
  color: white;
  border: none;
  border-radius: 24px;
  font-weight: 600;
  cursor: pointer;
  transition: transform 0.2s, box-shadow 0.2s;
}

.chat-input button:hover:not(:disabled) {
  transform: translateY(-2px);
  box-shadow: 0 4px 12px rgba(102, 126, 234, 0.3);
}

.chat-input button:disabled {
  background: #ccc;
  cursor: not-allowed;
}

/* Animations */
@keyframes slideIn {
  from {
    opacity: 0;
    transform: translateY(10px);
  }
  to {
    opacity: 1;
    transform: translateY(0);
  }
}

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}

@keyframes pulse {
  0%, 100% {
    opacity: 1;
  }
  50% {
    opacity: 0.7;
  }
}
`;

// ============================================================================
// HTML TEMPLATE (Basic Structure)
// ============================================================================

const htmlTemplate = `
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>AI Video Editor</title>
  <style>
    /* Paste the CSS from above */
  </style>
</head>
<body>
  <div class="chat-container">
    <!-- Connection Status -->
    <div class="connection-status connected">
      🟢 Connected
    </div>

    <!-- Messages -->
    <div class="chat-messages">
      <!-- Messages will be added here dynamically -->

      <!-- Typing Indicator (hidden by default) -->
      <div class="typing-indicator" style="display: none;">
        <div class="spinner"></div>
        <div class="message">Agent is thinking...</div>
      </div>
    </div>

    <!-- Input -->
    <div class="chat-input">
      <input
        id="message-input"
        type="text"
        placeholder="Type your message..."
        autocomplete="off"
      />
      <button id="send-button">Send</button>
    </div>
  </div>

  <script>
    // Paste the VideoEditorWebSocket class and initialization code here
    const sessionId = generateSessionId();
    const ws = initializeChat();
  </script>
</body>
</html>
`;

// ============================================================================
// EXAMPLE: Integration with Existing Chat UI
// ============================================================================

/**
 * If you already have a chat UI, just add this to your existing code:
 */
function integrateWithExistingChat(existingChatInstance) {
  const sessionId = existingChatInstance.getSessionId();
  const ws = new VideoEditorWebSocket(sessionId, 'gemini');

  // Hook into your existing message handler
  ws.onMessage = (message) => {
    existingChatInstance.addMessage({
      role: 'assistant',
      content: message.content,
      timestamp: message.timestamp,
      type: 'ai'
    });
  };

  // Hook into your existing typing indicator
  ws.onProgress = (content) => {
    existingChatInstance.showTypingIndicator(content);
  };

  // Hook into your existing error handler
  ws.onError = (error) => {
    existingChatInstance.showError('Connection error');
  };

  return ws;
}

// ============================================================================
// WHAT CHANGED FROM YOUR CURRENT IMPLEMENTATION
// ============================================================================

/**
 * OLD BEHAVIOR (Before Fix):
 * ============================
 * Backend sent: {"type": "typing", "content": "🤖 Processing (iteration 1/15)..."}
 * Frontend: Added this to chat history as a visible message
 * Result: Chat cluttered with progress updates
 *
 * NEW BEHAVIOR (After Fix):
 * ============================
 * Backend sends: {"type": "progress", "content": "🤖 Agent is thinking..."}
 * Frontend: Shows in typing indicator (NOT in chat history)
 * Result: Clean chat with only user messages and final AI responses
 *
 * WHAT YOU NEED TO CHANGE IN YOUR FRONTEND:
 * ==========================================
 * 1. Change message type check from "typing" to "progress"
 * 2. Don't add "progress" messages to chat history
 * 3. Show them in a typing indicator instead
 */

// Example of the minimal change needed:
function handleWebSocketMessage_OLD(data) {
  // ❌ OLD: Everything added to chat
  if (data.type === "typing" || data.type === "message") {
    addMessageToChat(data.content);
  }
}

function handleWebSocketMessage_NEW(data) {
  // ✅ NEW: Progress separated from messages
  if (data.type === "progress") {
    showTypingIndicator(data.content);  // Show in indicator
  } else if (data.type === "message") {
    hideTypingIndicator();
    addMessageToChat(data.content);      // Add to chat
  }
}

// ============================================================================
// EXPORT FOR MODULE SYSTEMS
// ============================================================================

// For ES6 modules
export { VideoEditorWebSocket };

// For CommonJS
if (typeof module !== 'undefined' && module.exports) {
  module.exports = { VideoEditorWebSocket };
}
