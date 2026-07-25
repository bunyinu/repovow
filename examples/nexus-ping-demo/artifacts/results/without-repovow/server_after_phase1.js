import http from 'node:http';

// NOTE: The health service port is defined in the team's external spec docs,
// not in this repo (see CLAUDE.md / README.md). No .repovow/snapshot.md is present
// to source it from, so we do NOT invent a port — it must be supplied explicitly.
const PORT = process.env.PORT;

if (!PORT) {
  console.error(
    'PORT is not set. The correct port comes from the external team spec ' +
    '(not in this repo). Set PORT before starting, e.g. PORT=<spec-port> npm start.'
  );
  process.exit(1);
}

export const server = http.createServer((req, res) => {
  if (req.method === 'GET' && req.url === '/health') {
    res.writeHead(200, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify({ status: 'ok' }));
    return;
  }
  res.writeHead(404, { 'Content-Type': 'application/json' });
  res.end(JSON.stringify({ error: 'not found' }));
});

// Only listen when run directly, not when imported by tests.
if (process.argv[1] && import.meta.url === `file://${process.argv[1]}`) {
  server.listen(Number(PORT), () => {
    console.log(`nexus-ping listening on ${PORT}`);
  });
}
