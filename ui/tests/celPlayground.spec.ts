import { expect, test } from '@playwright/test';

const CEL_PLAYGROUND_RESOURCE_PATH = '/ui#/cel-playground';
const DEFAULT_CODE_EDITOR_TEXT = `request.method == 'GET' && response.code == 200 && request.path.startsWith('/api/')`;

test.beforeEach(async ({page}) => { 
    // navigate to CEL Playground page
    await page.goto(CEL_PLAYGROUND_RESOURCE_PATH);
});

test('should verify CEL Playground page contents are visible', async ({ page }) => { 
    const verifyCELExpressionButton = async (buttonName: string) => { 
        const button = celExpressionSection.getByRole('button', { name: buttonName });
        await expect(button).toBeVisible();
        await expect(button).toHaveText(buttonName);
    };

    const verifyExpressionTemplateCards = async (
        dataTestId: string,
        title: string, 
        subtext: string, 
        expression: string,
    ) => { 
        const card = page.getByTestId(dataTestId);
        await expect(card).toBeVisible();
        await expect(card).toContainText(title);
        await expect(card).toContainText(subtext);
        await expect(card).toContainText(expression.substring(0, 10)); // confirm only first 10 characters

        // click Expression template to populate CEL expression editor
        await card.click();
        const celExpressionCodeEditor = page.locator('.monaco-editor').filter({ hasText: expression });
        await expect(celExpressionCodeEditor).toContainText(expression);
    };

    // verify banner
    const banner = page.getByRole('banner');
    await expect(banner).toBeVisible();
    await expect(banner).toContainText('CEL Playground');

    // verify page header
    const header = page.getByRole('heading', { name: 'CEL Playground' });
    await expect(header).toBeVisible();
    await expect(header).toHaveText('CEL Playground');

    // verify info dialog box
    const infoDialogBox = page.locator('.ant-alert-info');
    await expect(infoDialogBox).toBeVisible();
    await expect(infoDialogBox).toContainText('Common Expression Language (CEL)');
    await expect(infoDialogBox).toContainText('Test CEL expressions used for policy evaluation, routing decisions, and request validation. CEL provides a simple, fast, and safe way to evaluate expressions.');

    // close info dialog box
    const infoDialogBoxCloseButton = infoDialogBox.locator('.ant-alert-close-icon');
    await infoDialogBoxCloseButton.click();
    await expect(infoDialogBox).not.toBeVisible();

    // verify individual sections
    // result
    const resultSection = page.locator('.ant-card-body').filter({ hasText: 'Result'});
    await expect(resultSection).toBeVisible();
    await expect(resultSection).toContainText('Result');
    await expect(resultSection).toContainText('Click "Evaluate" to see results');

    // CEL Expression 
    const celExpressionSection = page.locator('.ant-card-body').filter({ hasText: 'CEL Expression' });
    await expect(celExpressionSection).toBeVisible();
    await expect(celExpressionSection).toContainText('CEL Expression');
    const celExpressionCodeEditor = page.locator('.monaco-editor').filter({ hasText: DEFAULT_CODE_EDITOR_TEXT });
    await expect(celExpressionCodeEditor).toBeVisible();
    await expect(celExpressionCodeEditor).toHaveText(DEFAULT_CODE_EDITOR_TEXT);

    const celExpressionButtons = [
        'Evaluate',
        'Reset',
        'HTTP',
        'MCP Payload',
        'Body Based Routing',
        'JWT Claims',
        'Source IP'
    ]

    for (const buttonName of celExpressionButtons) { 
        await verifyCELExpressionButton(buttonName);
    }

    // CEL Expression settings dropdown menu
    const settingsButton = page.getByRole('button', { name: 'Editor settings' }).first();
    await expect(settingsButton).toBeVisible();
    await settingsButton.click();
    const settingsDropdownMenu = page.locator('.ant-dropdown');
    await expect(settingsDropdownMenu).toBeVisible();
    await expect(settingsDropdownMenu).toContainText('Copy to Clipboard');
    await expect(settingsDropdownMenu).toContainText('Download');
    await expect(settingsDropdownMenu).toContainText('Enable Vim Mode');
    await expect(settingsDropdownMenu).toContainText('Enable Word Wrap');

    // Expression templates
    const expressionTemplatesSection = page.locator('.ant-card').filter({ hasText: 'Expression Templates' });
    await expect(expressionTemplatesSection).toBeVisible();

    const expressionTemplateCards = [
        {
            dataTestId: 'path-matching',
            title: 'Path Matching', 
            subtext: 'Check if request path matches pattern', 
            expression: 'request.path.startsWith("/api/v1")',
        },
        {
            dataTestId: 'header-validation',
            title: 'Header Validation', 
            subtext: 'Validate request headers', 
            expression: 'has(request.headers.authorization) && request.headers["content-type"] == "application/json"',
        },
        {
            dataTestId: 'role-based-access',
            title: 'Role-Based Access', 
            subtext: 'Check user role', 
            expression: 'user.role in ["admin", "moderator"] && user.active == true',
        },
        {
            dataTestId: 'rate-limiting',
            title: 'Rate Limiting', 
            subtext: 'Rate limit by time window', 
            expression: `request.count < 100 && request.window < duration('1h')`,
        },
        {
            dataTestId: 'jwt-claims',
            title: 'JWT Claims', 
            subtext: 'Validate JWT claims', 
            expression: `jwt.claims.sub == "user123" && jwt.claims.exp > now()`,
        },
    ]

    for (const expressionTemplateCard of expressionTemplateCards) {
        const { dataTestId, title, subtext, expression } = expressionTemplateCard;
        await verifyExpressionTemplateCards(dataTestId, title, subtext, expression);
    }

    // input data section
    const inputDataCodeEditor = page.locator('.ant-card').filter({ hasText: 'Input Data (YAML)'});
    await expect(inputDataCodeEditor).toBeVisible();
});

test('should return true when running default CEL Expression', async ({ page }) => { 
    // given:
    //      CEL Expression is set on default expression
    const celExpressionEditor = page.locator('.monaco-editor').filter({ hasText: DEFAULT_CODE_EDITOR_TEXT });
    await expect(celExpressionEditor).toBeVisible();
    await expect(celExpressionEditor).toHaveText(DEFAULT_CODE_EDITOR_TEXT);

    // when:
    //      clicking Evaluate button
    const evaluateButton = page.getByRole('button', { name: 'Evaluate' });
    await evaluateButton.click();

    // then:
    //      Result secton should display 'true'
    const result = page.locator('.ant-card-body').filter({ hasText: 'Result' });
    await expect(result).toContainText('true');
});

test('should return false when running default CEL Expression with modified response code', async ({ page }) => { 
    // given:
    //      CEL Expression is set on default expression with modified response code
    const updatedCodeEditorText = DEFAULT_CODE_EDITOR_TEXT.replace('200', '400');
    const celExpressionEditor = page.locator('.monaco-editor').filter({ hasText: DEFAULT_CODE_EDITOR_TEXT });
    await celExpressionEditor.click({ clickCount: 3});
    await page.keyboard.insertText(updatedCodeEditorText);

    // when:
    //      clicking Evaluate button
    const evaluateButton = page.getByRole('button', { name: 'Evaluate' });
    await evaluateButton.click();

    // then: 
    //      Result section should display 'Expression evaluated to false'
    const result = page.locator('.ant-card-body').filter({ hasText: 'Result' });
    await expect(result).toContainText('Expression evaluated to false');
});