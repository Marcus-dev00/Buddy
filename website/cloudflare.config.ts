export default {
  name: "buddy-companion",
  main: "src/worker.js",
  compatibility_date: "2026-11-01",
  assets: {
    directory: "./public",
    binding: "ASSETS",
  },
  kv_namespaces: [
    {
      binding: "BUDDY_DIR",
      id: "69c10b90f255418b8107a5866c60fde4",
    },
  ],
};
