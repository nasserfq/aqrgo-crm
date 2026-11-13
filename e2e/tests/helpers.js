const fs = require('fs');
const { expect } = require('@playwright/test');

/** A fresh address per test, so tests never share data or hit the code rate limit. */
function freshEmail(tag) {
  return `e2e-${tag}-${Date.now()}-${Math.floor(Math.random() * 1e6)}@example.com`;
}

/** Waits for the sign-in code the app logged for this address. */
async function codeFor(email) {
  const log = process.env.E2E_SERVER_LOG;
  for (let i = 0; i < 50; i++) {
    const text = fs.readFileSync(log, 'utf8');
    const at = text.lastIndexOf(email);
    const m = at >= 0 && text.slice(at).match(/sign-in code is (\d{6})/);
    if (m) return m[1];
    await new Promise((r) => setTimeout(r, 200));
  }
  throw new Error(`no sign-in code logged for ${email}`);
}

/** Signs in through the email-code form and lands on the table. */
async function signIn(page, email) {
  await page.goto('/login');
  await page.getByLabel('البريد الإلكتروني').fill(email);
  await page.getByRole('button', { name: 'أرسل رمز الدخول' }).click();
  await expect(page.getByText('أرسلنا رمزاً من 6 أرقام')).toBeVisible();
  await page.getByLabel('رمز الدخول').fill(await codeFor(email));
  await page.getByRole('button', { name: 'دخول', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'جدول العروض' })).toBeVisible();
}

/** Fills the new-listing form by field label and saves it. */
async function addListing(page, fields, status) {
  await page.getByRole('link', { name: /إضافة عرض/ }).first().click();
  await expect(page.getByRole('heading', { name: 'عرض جديد' })).toBeVisible();
  for (const [label, value] of Object.entries(fields)) {
    await page.getByLabel(label, { exact: true }).fill(value);
  }
  if (status) await page.locator('select[name=status]').selectOption({ label: status });
  await page.getByRole('button', { name: 'حفظ' }).click();
  await expect(page.getByRole('heading', { name: 'جدول العروض' })).toBeVisible();
}

/** The page must never scroll sideways (the table scrolls inside its own box). */
async function expectNoSidewaysScroll(page) {
  const [client, scroll] = await page.evaluate(() => [document.documentElement.clientWidth, document.documentElement.scrollWidth]);
  expect(scroll, 'page is wider than the screen').toBeLessThanOrEqual(client);
}

module.exports = { freshEmail, codeFor, signIn, addListing, expectNoSidewaysScroll };
