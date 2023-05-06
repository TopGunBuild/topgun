const {createServer} = require('../dist/server.js');

const server = createServer({
    port: 8765
});

console.log('TopGun Server started on port ' + server.options.port);