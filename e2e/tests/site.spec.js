// End-to-end checks of aqrgo.com, run against the real app and database.
const { test, expect } = require('@playwright/test');
const { freshEmail, signIn, addListing, expectNoSidewaysScroll } = require('./helpers');

test.describe('visitors', () => {
  test('see the landing page with a way in', async ({ page }) => {
    await page.goto('/');
    await expect(page.getByRole('heading', { level: 1 })).toContainText('تيليجرام');
    await expect(page.getByRole('heading', { name: 'مكتبك كله داخل تيليجرام' })).toBeVisible();
    await expectNoSidewaysScroll(page);
    await page.getByRole('link', { name: 'ابدأ الآن' }).first().click();
    await expect(page).toHaveURL(/\/login$/);
    await expect(page.getByRole('link', { name: /الدخول بحساب Google/ })).toBeVisible();
  });

  test('are sent to sign in from private pages', async ({ page }) => {
    await page.goto('/listings/new');
    await expect(page).toHaveURL(/\/login$/);
    await page.goto('/account');
    await expect(page).toHaveURL(/\/login$/);
  });

  test('cannot sign in with a wrong code', async ({ page }) => {
    await page.goto('/login');
    await page.getByLabel('البريد الإلكتروني').fill(freshEmail('wrong'));
    await page.getByRole('button', { name: 'أرسل رمز الدخول' }).click();
    await page.getByLabel('رمز الدخول').fill('000000');
    await page.getByRole('button', { name: 'دخول', exact: true }).click();
    await expect(page.locator('.error')).toBeVisible();
    await expect(page.getByRole('heading', { name: 'جدول العروض' })).toHaveCount(0);
  });

  test('always get the current stylesheet', async ({ page, request }) => {
    await page.goto('/');
    const href = await page.locator('link[rel=stylesheet][href^="/static/app.css"]').getAttribute('href');
    expect(href).toMatch(/\/static\/app\.css\?v=\d+$/);
    const css = await request.get(href);
    expect(css.ok()).toBeTruthy();
    expect(css.headers()['cache-control']).toContain('immutable');
    expect(await css.text()).toContain('--accent');
  });
});

test.describe('a signed-in user', () => {
  test('starts with an empty table that says what to do', async ({ page }) => {
    await signIn(page, freshEmail('empty'));
    await expect(page.getByText('لا توجد عروض هنا بعد')).toBeVisible();
    await expect(page.locator('.count b')).toHaveText('0');
    await expectNoSidewaysScroll(page);
  });

  test('adds, finds, edits and deletes a listing', async ({ page }) => {
    await signIn(page, freshEmail('crud'));

    await addListing(page, {
      'نوع العرض': 'أرض للبيع',
      'رقم المخطط': '٢٤٥٦',
      'رقم القطعة': '12 و 13',
      'المساحة': '625م',
      'الموقع': 'حي الملقا، الرياض',
      'السعر': 'على السوم',
      'بيانات البائع': 'أبو فهد 0551234870',
    });
    await addListing(page, {
      'نوع العرض': 'عمارة للبيع',
      'الموقع': 'حي الخليج، الرياض',
      'عدد الغرف': '44',
      'الوحدات': '7 شقق',
      'تفاصيل إضافية': 'مصعد راكب\nمباشر مع الوكيل',
    }, 'محجوز');

    const rows = page.locator('#rows tr');
    await expect(rows).toHaveCount(2);
    await expect(page.locator('.count b')).toHaveText('2');
    // Arabic-Indic digits are stored as plain digits.
    await expect(rows.filter({ hasText: 'أرض للبيع' })).toContainText('2456');
    await expect(rows.filter({ hasText: 'عمارة للبيع' }).locator('.pill')).toHaveText('محجوز');

    // Live search narrows the table without reloading the page.
    await page.getByLabel('بحث').fill('الملقا');
    await expect(rows).toHaveCount(1);
    await expect(rows.first()).toContainText('أرض للبيع');
    await page.getByLabel('بحث').fill('');
    await expect(rows).toHaveCount(2);

    // Status filter.
    await page.locator('#status').selectOption({ label: 'محجوز' });
    await expect(rows).toHaveCount(1);
    await expect(rows.first()).toContainText('عمارة للبيع');
    await page.locator('#status').selectOption({ label: 'كل الحالات' });
    await expect(rows).toHaveCount(2);

    // Open a row, change it, save.
    await rows.filter({ hasText: 'أرض للبيع' }).click();
    await expect(page.getByRole('heading', { name: /تعديل العرض/ })).toBeVisible();
    await page.getByLabel('السعر', { exact: true }).fill('1.2 مليون');
    await page.locator('select[name=status]').selectOption({ label: 'مباع' });
    await page.getByRole('button', { name: 'حفظ' }).click();
    const land = page.locator('#rows tr', { hasText: 'أرض للبيع' });
    await expect(land).toContainText('1.2 مليون');
    await expect(land.locator('.pill')).toHaveText('مباع');

    // Export follows the current filters.
    const [download] = await Promise.all([
      page.waitForEvent('download'),
      page.getByRole('link', { name: 'تصدير Excel' }).click(),
    ]);
    const csv = require('fs').readFileSync(await download.path(), 'utf8');
    expect(csv).toContain('عمارة للبيع');
    expect(csv).toContain('1.2 مليون');

    // Delete asks first, then removes the row.
    await land.click();
    page.once('dialog', (d) => d.accept());
    await page.getByRole('button', { name: 'حذف العرض' }).click();
    await expect(page.locator('#rows tr')).toHaveCount(1);
    await expect(page.locator('#rows')).not.toContainText('أرض للبيع');
  });

  test('only ever sees their own listings', async ({ browser }) => {
    const a = await (await browser.newContext()).newPage();
    await signIn(a, freshEmail('owner'));
    await addListing(a, { 'نوع العرض': 'فيلا للإيجار', 'الموقع': 'النرجس' });
    const ownUrl = await a.locator('#rows tr').first().evaluate((tr) => tr.getAttribute('onclick').match(/'(.+)'/)[1]);

    const b = await (await browser.newContext()).newPage();
    await signIn(b, freshEmail('other'));
    await expect(b.locator('#rows')).not.toContainText('فيلا للإيجار');
    // Opening someone else's listing by its address just returns you to your own table.
    await b.goto(ownUrl);
    await expect(b).toHaveURL(/\/$/);
    await expect(b.getByRole('heading', { name: 'جدول العروض' })).toBeVisible();
    await expect(b.locator('body')).not.toContainText('فيلا للإيجار');
  });

  test('rejects a listing with no details', async ({ page }) => {
    await signIn(page, freshEmail('blank'));
    await page.getByRole('link', { name: /إضافة عرض/ }).first().click();
    await page.getByRole('button', { name: 'حفظ' }).click();
    await expect(page.locator('.error')).toBeVisible();
  });

  test('can open the account page and sign out', async ({ page }) => {
    const email = freshEmail('acct');
    await signIn(page, email);
    await page.getByRole('link', { name: /حسابي/ }).click();
    await expect(page.getByRole('heading', { name: 'حسابي' })).toBeVisible();
    await expect(page.getByText(email)).toBeVisible();
    await expectNoSidewaysScroll(page);
    await page.getByRole('button', { name: 'خروج' }).click();
    await page.goto('/listings/new');
    await expect(page).toHaveURL(/\/login$/);
  });
});
