async function main(): Promise<void> {
  const input = await Bun.stdin.text();
  const newline = input.indexOf("\n");
  const k = Number(input.slice(0, newline));
  const text = input.slice(newline + 1).toLowerCase();

  const counts = new Map<string, number>();
  for (const [word] of text.matchAll(/[a-z]+/g)) {
    counts.set(word, (counts.get(word) ?? 0) + 1);
  }

  // Words are ASCII, so comparing them as strings compares their bytes.
  const ranked = [...counts].sort(
    ([wordA, countA], [wordB, countB]) => countB - countA || (wordA < wordB ? -1 : 1),
  );
  const out = ranked
    .slice(0, k)
    .map(([word, count]) => `${word} ${count}\n`)
    .join("");
  if (out) {
    await Bun.write(Bun.stdout, out);
  }
}

await main();
