const used = process.memoryUsage().heapUsed / 1024 / 1024;

const nodes = [];

function append() {
  for (let i = 0; i < 10_000_000; i++) {
    // Your code here
    nodes.push({
      id: i,
      hash: new Date().getTime()
    });
  }
}
append();

console.log(`The script uses approximately ${Math.round(used * 100) / 100} MB, result length: ${nodes.length}`);
