import WebSocket from 'ws';

const [,, sessionId, token, workflowId] = process.argv;
if (!sessionId || !token || !workflowId) {
  console.error('Usage: node trigger_agent.mjs <session_id> <token> <workflow_id>');
  process.exit(1);
}

const url = `wss://video-sync-723463981172.us-central1.run.app/ws?session=${sessionId}&token=${token}&workflow_id=${workflowId}`;
const ws = new WebSocket(url);

ws.on('open', () => {
  console.log('CONNECTED');
  // Send an initial message to trigger the agent
  const msg = JSON.stringify({type: 'user_message', content: 'Generate the sample as requested. Start working on it.'});
  ws.send(msg);
  console.log('SENT:', msg);
});

ws.on('message', (data) => {
  const text = data.toString();
  console.log('MSG:', text);
  try {
    const parsed = JSON.parse(text);
    // If we detect completion, exit
    if (parsed.type === 'agent_complete' || parsed.type === 'workflow_complete' || parsed.type === 'error') {
      console.log('DONE');
      ws.close();
    }
  } catch {}
});

ws.on('close', () => {
  console.log('CLOSED');
  process.exit(0);
});

ws.on('error', (err) => {
  console.error('ERROR:', err.message);
  process.exit(1);
});

// Timeout after 10 minutes
setTimeout(() => {
  console.log('TIMEOUT');
  ws.close();
  process.exit(0);
}, 600000);
