# TopGun Deployment Walkthrough

**Date**: [YYYY-MM-DD]
**Deployer**: [Name]
**Version**: [Git Commit / Tag]

## 1. Pre-Deployment Check
- [ ] Local build successful?
- [ ] Tests passed?
- [ ] VPS accessible via SSH?
- [ ] Docker installed on VPS?

## 2. Deployment Execution
### Step 1: Transfer
- [ ] Files transferred to VPS (`docker-compose.yml`, `.env`, etc.)
- [ ] Method used: [git pull / scp / docker registry]

### Step 2: Configuration
- [ ] `.env` file created/updated with production secrets.
- [ ] `docker-compose.yml` verified (includes log rotation).

### Step 3: Launch
- [ ] Command run: `docker-compose up -d`
- [ ] Output observed:
  ```
  [Paste output snippet here]
  ```

## 3. Verification & Testing

### Service Status
- [ ] `docker-compose ps` output:
  ```
  [Paste output here]
  ```

### Logs Check
- [ ] `docker-compose logs --tail=50 server`
- [ ] Any errors? [Yes/No]
- [ ] "TopGun Server Starting" message visible? [Yes/No]

### Functional Test
- [ ] HTTP Health Check: `curl -v http://localhost:8080/`
  - Result: [200 OK / 404 / Error]
- [ ] Database Connection:
  - Verified in logs? [Yes/No]

## 4. Issues Encountered
| Issue | Resolution |
|-------|------------|
| [e.g. Port conflict] | [Changed port in docker-compose] |

## 5. Sign-off
- [ ] Deployment successful and verified.
