# Email Notification Setup for GitHub CI Benchmarks

This document explains how to configure email notifications for the AgentGateway benchmark CI system.

## Overview

The benchmark workflows can send professional email notifications to maintainers when benchmarks complete, fail, or are cancelled. The system includes:

- **HTML email notifications** with detailed benchmark results
- **Performance summaries** attached as text files
- **Baseline update notifications** when industry baselines change
- **Fallback GitHub issues** if email delivery fails
- **Professional styling** with status indicators and action buttons

## Required GitHub Secrets

To enable email notifications, repository maintainers must configure the following secrets in the GitHub repository settings:

### 1. NOTIFICATION_EMAIL_USER
The email address used to send notifications (SMTP username).

**Example:** `agentgateway-ci@company.com`

### 2. NOTIFICATION_EMAIL_PASSWORD
The password or app-specific password for the notification email account.

**Security Note:** Use app-specific passwords when available (Gmail, Outlook, etc.)

### 3. MAINTAINER_EMAILS
Comma-separated list of maintainer email addresses to receive notifications.

**Example:** `maintainer1@company.com,maintainer2@company.com,team-lead@company.com`

## Email Provider Configuration

### Gmail Setup
1. Enable 2-factor authentication on the Gmail account
2. Generate an app-specific password:
   - Go to Google Account settings
   - Security → 2-Step Verification → App passwords
   - Generate password for "Mail"
3. Use the app password as `NOTIFICATION_EMAIL_PASSWORD`

### Outlook/Office 365 Setup
1. Enable 2-factor authentication
2. Generate an app password:
   - Go to Security settings
   - Additional security verification → App passwords
   - Create app password for "Mail"
3. Use `smtp-mail.outlook.com` (port 587) if needed

### Custom SMTP Server
To use a different SMTP server, modify the workflow file:

```yaml
- name: Send Email Notification
  uses: dawidd6/action-send-mail@v3
  with:
    server_address: your-smtp-server.com  # Change this
    server_port: 587                      # Change if needed
    username: ${{ secrets.NOTIFICATION_EMAIL_USER }}
    password: ${{ secrets.NOTIFICATION_EMAIL_PASSWORD }}
    # ... rest of configuration
```

## Setting Up GitHub Secrets

1. Navigate to your repository on GitHub
2. Go to **Settings** → **Secrets and variables** → **Actions**
3. Click **New repository secret**
4. Add each required secret:

   | Secret Name | Example Value |
   |-------------|---------------|
   | `NOTIFICATION_EMAIL_USER` | `agentgateway-ci@gmail.com` |
   | `NOTIFICATION_EMAIL_PASSWORD` | `your-app-specific-password` |
   | `MAINTAINER_EMAILS` | `dev1@company.com,dev2@company.com` |

## Email Content

### Success Notifications
- ✅ Status indicator with benchmark configuration
- Performance summary with key metrics
- Links to detailed results and artifacts
- Baseline update information (if applicable)
- Next steps for result analysis

### Failure Notifications
- ❌ Status indicator with error context
- Configuration details for debugging
- Links to workflow logs and error details
- Troubleshooting guidance
- Escalation recommendations

### Baseline Update Notifications
- 📊 Industry baseline changes detected
- Source information (TechEmpower, vendor releases, etc.)
- Impact assessment and confidence scores
- Integration with benchmark results

## Fallback Mechanisms

If email delivery fails, the system automatically:

1. **Creates a GitHub issue** with the notification content
2. **Labels the issue** with `benchmark`, `notification`, `ci`
3. **Includes all relevant details** from the failed email
4. **Notifies maintainers** via GitHub's issue notification system

## Testing Notifications

To test the notification system:

1. Ensure all secrets are configured correctly
2. Trigger a manual benchmark via GitHub Actions UI
3. Enable the "Send notification to maintainers" option
4. Check email inboxes and GitHub issues for delivery

### Test Command
```bash
# Trigger via PR comment (maintainers only)
/benchmark http quick 30s
```

## Troubleshooting

### Common Issues

**Email not received:**
- Check spam/junk folders
- Verify SMTP credentials and server settings
- Check GitHub Actions logs for error messages
- Look for fallback GitHub issues

**Authentication failures:**
- Ensure app-specific passwords are used
- Verify 2-factor authentication is enabled
- Check SMTP server and port configuration

**Missing baseline updates:**
- Verify Python dependencies are installed
- Check external API rate limits
- Review baseline update script logs

### Debug Steps

1. **Check workflow logs:**
   - Go to Actions → Benchmark workflow run
   - Review "Send Email Notification" step logs

2. **Verify secrets:**
   - Ensure all required secrets are set
   - Check for typos in secret names

3. **Test SMTP connection:**
   - Use a simple email test in a separate workflow
   - Verify server connectivity and authentication

## Security Considerations

- **Use app-specific passwords** instead of main account passwords
- **Limit secret access** to necessary workflows only
- **Regularly rotate** email credentials
- **Monitor notification logs** for suspicious activity
- **Use dedicated email accounts** for CI notifications

## Customization

### Email Templates
The HTML email template can be customized by modifying the `html_body` section in the workflow file. The template includes:

- Professional styling with CSS
- Status-specific color coding
- Responsive design for mobile devices
- Action buttons for quick access
- Baseline update sections

### Notification Frequency
Notifications are sent for:
- ✅ Successful benchmark completions
- ❌ Benchmark failures
- ⚠️ Cancelled or skipped benchmarks
- 📊 Baseline updates (when detected)

### Additional Recipients
To add more recipients, update the `MAINTAINER_EMAILS` secret with additional comma-separated email addresses.

## Support

For issues with the notification system:

1. Check this documentation first
2. Review GitHub Actions workflow logs
3. Test with a minimal configuration
4. Create a GitHub issue with error details
5. Contact repository maintainers for assistance

---

**Note:** Email notifications are optional and can be disabled by setting `notify_maintainers: false` in the workflow trigger or PR comment.
