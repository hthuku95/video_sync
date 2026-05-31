import WebSocket from 'ws';
import crypto from 'crypto';

const [,, token, ...promptParts] = process.argv;
if (!token) {
  console.error('Usage: node test_fresh.mjs <token> "<prompt>"');
  process.exit(1);
}

const prompt = promptParts.join(' ');
const sessionId = crypto.randomUUID();
const url = `wss://video-sync-723463981172.us-central1.run.app/ws?session=${sessionId}&token=${token}`;

console.log('Session:', sessionId);
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
    const content = (parsed.content || '').substring(0, 500);
    console.log(`\n[${type}] ${content}`);
    if (type === 'message') {
      console.log('\n=== FINAL RESULT ===');
      ws.close();
    }
  } catch {
    console.log('RAW:', text.substring(0, 200));
  }
});

ws.on('close', () => console.log('\nCLOSED after', msgCount, 'messages'));
ws.on('error', (err) => console.error('ERROR:', err.message));
setTimeout(() => { console.log('TIMEOUT'); process.exit(0); }, 600000);
