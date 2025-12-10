const { createPublicKey } = require('crypto');
const https = require('https');

const url = process.argv[2];

if (!url) {
  console.error('Usage: node scripts/get-clerk-key.js <jwks_url>');
  console.error('Example: node scripts/get-clerk-key.js https://your-domain.clerk.accounts.dev/.well-known/jwks.json');
  process.exit(1);
}

console.error(`Fetching JWKS from ${url}...`);

https.get(url, (res) => {
  if (res.statusCode !== 200) {
    console.error(`Failed to fetch JWKS: Status Code ${res.statusCode}`);
    res.resume();
    process.exit(1);
  }

  let data = '';
  res.on('data', (chunk) => {
    data += chunk;
  });

  res.on('end', () => {
    try {
      const jwks = JSON.parse(data);
      if (!jwks.keys || jwks.keys.length === 0) {
        console.error('No keys found in JWKS response.');
        process.exit(1);
      }

      // Usually the first key is the signing key, or look for 'use': 'sig'
      const sigKey = jwks.keys.find(k => k.use === 'sig') || jwks.keys[0];

      if (!sigKey) {
         console.error('No signing key found.');
         process.exit(1);
      }

      const publicKey = createPublicKey({ key: sigKey, format: 'jwk' });
      const pem = publicKey.export({ type: 'spki', format: 'pem' });
      
      console.log('\nHere is your Public Key (PEM):\n');
      console.log(pem);
      console.log('\nCopy the content above (including BEGIN and END lines) into your JWT_SECRET environment variable.');
      
    } catch (e) {
      console.error('Error parsing JWKS:', e.message);
      process.exit(1);
    }
  });
}).on('error', (e) => {
  console.error(`Got error: ${e.message}`);
  process.exit(1);
});

