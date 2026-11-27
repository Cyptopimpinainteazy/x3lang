# CI diagnosis and external build remediation

Previous PR273 head: `32375b75dae8610e7fe95f603c1467dc0c360c47`.
All 19 executed checks passed; one manual advisory comparison was skipped.

The separate legacy Vercel project `prj_0HRd4FQA9Bva1RxZuPcshRnkPYxM`
(`v0-new-project-ytafkf4cr0l`) failed because it ran `bun install` at the Python/Rust
repository root as Next.js: `Bun could not find a package.json file to install from`.

An authorized build-only configuration correction was applied and read back:

| Setting | Old | New |
|---|---|---|
| framework | nextjs | vite |
| rootDirectory | null | x3-lang/forge/web |
| installCommand | bun install | null (framework/package-lock default) |
| buildCommand | bun run build | null (package build script) |
| outputDirectory | null | null (Vite defaults to dist) |

These match the separate successful x3lang project's build fields. The actual app has an
npm lockfile and builds with `tsc -b && vite build`. Domains, protection, Node 22, environment
variables, billing, repository integration and production branch were not changed.
No deployment was manually created or promoted. A new ordinary PR push must verify the fix.

RecurseML's previous status was an external analysis error, not a code finding; it exposed
no usable failure annotations. Its next-head status must be checked separately rather than
marked green from local tests or another service's result.
