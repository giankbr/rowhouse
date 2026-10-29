// Conventional Commits: https://www.conventionalcommits.org
// e.g. "feat(grid): inline cell editing", "fix(mcp): reject EXPLAIN ANALYZE writes"
export default {
  extends: ["@commitlint/config-conventional"],
  rules: {
    "scope-enum": [
      1,
      "always",
      ["core", "mcp", "ai", "desktop", "ui", "grid", "editor", "drivers", "deps", "ci", "docs", "release"],
    ],
    "body-max-line-length": [0],
  },
};
