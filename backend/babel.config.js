// Lets Jest (via babel-jest) transform TypeScript for the test runner.
//  - @babel/preset-env: transforms ESM `import`/`export` to CommonJS
//    (targets the current Node, so it's fast and mostly just does modules).
//  - @babel/preset-typescript: strips type annotations.
// Type-checking is intentionally NOT performed during tests — run
// `npx tsc --noEmit` (`make lint`) in CI/build for that.
module.exports = {
  presets: [
    ['@babel/preset-env', { targets: { node: 'current' } }],
    '@babel/preset-typescript',
  ],
};
