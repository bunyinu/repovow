import { createServer } from 'node:http';

// No port documented in project requirements; defaulting to 3000 (override with PORT).
const PORT = Number(process.env.PORT) || 3000;

export const server = createServer((req, res) => {
  const url = new URL(req.url, `http://${req.headers.host}`);

  if (req.method === 'GET' && url.pathname === '/greet') {
    const name = url.searchParams.get('name') || 'World';
    res.writeHead(200, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify({ message: `Hello, ${name}!` }));
    return;
  }

  res.writeHead(404, { 'Content-Type': 'application/json' });
  res.end(JSON.stringify({ error: 'Not Found' }));
});

// Only listen when run directly, not when imported by tests.
if (process.argv[1] && import.meta.url === `file://${process.argv[1]}`) {
  server.listen(PORT, () => console.log(`Listening on ${PORT}`));
}
