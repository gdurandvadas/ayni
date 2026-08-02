export default [
  { ignores: ["**/*.ts"] },
  {
    files: ["**/*.js"],
    rules: {
      complexity: ["error", 12],
    },
  },
];
