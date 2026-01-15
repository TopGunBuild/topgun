# Secrets Directory

This directory contains secret files for Docker Compose Zero-Touch Setup.

## Usage

1. Create secret files (not tracked by git):

```bash
# Admin password for TopGun setup
echo "your-secure-password-here" > admin_password.txt

# Database password (if needed separately)
echo "db-password-here" > db_password.txt
```

2. Start with auto-setup profile:

```bash
docker compose --profile auto-setup up
```

## Security Notes

- **Never commit secret files to git** - the `.gitignore` prevents this
- Use strong passwords (minimum 8 characters)
- In production, use proper secrets management (Vault, AWS Secrets Manager, etc.)
- File permissions should be restrictive: `chmod 600 *.txt`

## File Format

- One secret per file
- No trailing whitespace (will be trimmed automatically)
- UTF-8 encoding
