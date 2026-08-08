# Project Context: Spec-Driven Development (SDD)

## Tech Stack
- **Backend:** Node.js (Express + TypeScript) managed via `npm`
- **Tooling:** TypeScript (strict), Zod (validation), Jest (tests)

## Pipeline
Run `make help` for targets. Order: domain → reqs → spec → review (PASS required) → code.

## Conventions
- Source lives in `backend/src/`; entrypoint `backend/src/app.ts`.
- Use strict TypeScript, Express router patterns, and Zod for request validation.
- Follow `prompts/` strictly. Do not invent requirements untraceable to an FR/NFR ID.
