import WebSocket from 'ws';

const [,, sessionId, token, workflowId, ...promptParts] = process.argv;
if (!sessionId || !token || !workflowId) {
  console.error('Usage: node test_service.mjs <session_id> <token> <workflow_id> <prompt>');
  process.exit(1);
}

const prompt = promptParts.join(' ') || 'Generate the sample as requested. Start working on it.';
const url = `wss://video-sync-723463981172.us-central1.run.app/ws?session=${sessionId}&token=${token}&workflow_id=${workflowId}`;

console.log('Connecting...');
const ws = new WebSocket(url);
let msgCount = 0;

ws.on('open', () => {
  console.log('CONNECTED');
  ws.send(JSON.stringify({type: 'user_message', content: prompt}));
  console.log('SENT prompt');
});

ws.on('message', (data) => {
  const text = data.toString();
  msgCount++;
  try {
    const parsed = JSON.parse(text);
    const type = parsed.type || '?';
    const content = (parsed.content || '').substring(0, 400);
    console.log(`[${type}] ${content}`);
    if (type === 'result' || type === 'error') {
      console.log('DONE');
      ws.close();
    }
  } catch {
    console.log('RAW:', text.substring(0, 200));
  }
});

ws.on('close', () => {
  console.log('CLOSED after', msgCount, 'messages');
  process.exit(0);
});
ws.on('error', (err) => { console.error('ERROR:', err.message); process.exit(1); });
setTimeout(() => { console.log('TIMEOUT'); ws.close(); process.exit(0); }, 600000);
