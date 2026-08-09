# AI-Accelerated SDD Pipeline
#
# Stages run headless through the Claude Code CLI. CLAUDE_FLAGS auto-accepts
# file edits so each stage can write its artifact without prompting.
# Override it if you prefer, e.g.:   make spec CLAUDE_FLAGS="claude -p"

CLAUDE_FLAGS := claude -p --permission-mode acceptEdits

.PHONY: help setup domain reqs spec review code lint test all

help: ## Show available targets
	@awk 'BEGIN {FS = ":.*##"; printf "SDD Pipeline — make targets:\n\n"} /^[a-zA-Z_-]+:.*##/ { printf "  \033[36m%-8s\033[0m %s\n", $$1, $$2 }' $(MAKEFILE_LIST)

setup: ## Install/sync backend dependencies (auto-detects stack)
	@if [ -f backend/pyproject.toml ]; then cd backend && uv sync; \
	elif [ -f backend/package.json ]; then cd backend && npm install; \
	elif [ -f backend/go.mod ]; then cd backend && go mod tidy; \
	else echo "No recognized backend in backend/" >&2; exit 1; fi

domain: ## Stage 1 - DOMAIN.md + CONTEXT_MAP.md from raw_idea.txt
	$(CLAUDE_FLAGS) "$$(cat prompts/p0_domain.txt)"

reqs: domain ## Stage 2 - REQUIREMENTS.md from the domain docs
	$(CLAUDE_FLAGS) "$$(cat prompts/p1_requirements.txt)"

spec: reqs ## Stage 3 - SPEC.md from requirements
	$(CLAUDE_FLAGS) "$$(cat prompts/p2_spec.txt)"

review: spec ## Stage 5 - PASS/FAIL gate on SPEC.md (blocks 'code' on FAIL)
	@mkdir -p docs/03-review
	$(CLAUDE_FLAGS) "$$(cat prompts/p3_review.txt)"
	@if head -n 1 docs/03-review/VERDICT.md 2>/dev/null | grep -qiE 'VERDICT:[[:space:]]*PASS'; then \
		touch docs/03-review/PASS; printf '\033[0;32m✓ Spec review PASSED.\033[0m\n'; \
	else \
		printf '\033[0;31m✗ Review did not PASS. Fix the blocking issues, re-run "make spec", then "make review".\033[0m\n' >&2; exit 1; \
	fi

code: review ## Stage 6 - implement SPEC.md (requires review PASS)
	$(CLAUDE_FLAGS) "$$(cat prompts/p4_code.txt)"

lint: ## Type-check backend TypeScript (tsc --noEmit)
	@if [ -f backend/package.json ]; then cd backend && npx tsc --noEmit; \
	else echo "No recognized backend in backend/" >&2; exit 1; fi

test: ## Run the backend test suite (auto-detects stack)
	@if [ -f backend/pyproject.toml ]; then cd backend && uv run pytest tests/ -v; \
	elif [ -f backend/package.json ]; then cd backend && npx jest; \
	elif [ -f backend/go.mod ]; then cd backend && go test ./... -v; \
	else echo "No recognized backend in backend/" >&2; exit 1; fi

all: setup code ## Full pipeline: setup -> domain -> ... -> code
	@printf '\n\033[0;32m✅ SDD pipeline complete.\033[0m\n'
