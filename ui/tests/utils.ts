import { expect, type Page } from "@playwright/test";

export const verifyLogPageContents = async (page: Page, title: string) => { 
    // verify banner
    const banner = page.getByRole('banner');
    await expect(banner).toBeVisible();
    await expect(banner).toContainText(title);

    // verify log search input
    const logSearchInput = page.getByTestId('log-viewer-search');
    await expect(logSearchInput).toBeVisible();

    // verify Wrap Text checkbox
    const wrapTextCheckbox = page.getByTestId('wrap-text-checkbox');
    await expect(wrapTextCheckbox).toBeVisible();
    await expect(wrapTextCheckbox).toBeChecked({ checked: false});

    // verify Log Viewer container
    const logViewer = page.getByTestId('log-viewer');
    await expect(logViewer).toBeVisible();

    // TODO: verify ChevronDown icon after scrolling up in logs
};