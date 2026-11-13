require('node:http').createServer((_request, response) => {
  response.writeHead(200, { 'content-type': 'text/plain', 'cache-control': 'no-store' });
  response.end('hoista-live-fly-v1');
}).listen(8080, '0.0.0.0');
