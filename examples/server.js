const {TGServer} = require('../dist/server.js');

const server = new TGServer({
    port: 8765
});

console.log('TopGun Server started on port ' + server.options.port);