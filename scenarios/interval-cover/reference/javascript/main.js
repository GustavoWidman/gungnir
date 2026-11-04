async function main() {
  const tokens = (await Bun.stdin.text()).split(/\s+/);
  const n = Number(tokens[0]);
  const q = Number(tokens[1]);

  // Coordinates span the full signed 64-bit range, which a Number cannot hold exactly.
  const starts = new BigInt64Array(n);
  const ends = new BigInt64Array(n);
  for (let i = 0; i < n; i++) {
    starts[i] = BigInt(tokens[2 + 2 * i]);
    ends[i] = BigInt(tokens[3 + 2 * i]);
  }
  starts.sort();
  ends.sort();

  const counts = new Array(q);
  for (let i = 0; i < q; i++) {
    const x = BigInt(tokens[2 + 2 * n + i]);
    // Intervals containing x: those starting at or before x, minus those ending before x.
    counts[i] = partitionPoint(starts, (s) => s <= x) - partitionPoint(ends, (e) => e < x);
  }
  if (q > 0) {
    await Bun.write(Bun.stdout, counts.join("\n") + "\n");
  }
}

/** The number of leading values of `sorted` that satisfy `pred`, which holds for a prefix. */
function partitionPoint(sorted, pred) {
  let lo = 0;
  let hi = sorted.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (pred(sorted[mid])) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}

await main();
