# trash2review

Holding pen for files the cleanup pipeline flagged as redundant, dead, or
superseded. **Nothing here is deleted** — it was moved via `git mv`, so it is
fully recoverable and still in git history.

## Restore a file

```
git mv trash2review/<mirrored/path>/<file> <original/path>/<file>
```

For example, if `src/old_thing.ts` was moved to `trash2review/src/old_thing.ts`:

```
git mv trash2review/src/old_thing.ts src/old_thing.ts
```

## Reviewing

Each entry should have a one-line reason in the pipeline's purge plan
(`.pipeline/purge-plan.md`). Review, restore anything you want to keep, then
empty this folder once you're satisfied. Files left here have no effect on the
running application.
