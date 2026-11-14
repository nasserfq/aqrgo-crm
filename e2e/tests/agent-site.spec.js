// An agent builds their public website (موقعي) and a visitor opens it.
const { test, expect } = require('@playwright/test');
const { freshEmail, signIn, addListing, expectNoSidewaysScroll } = require('./helpers');

test('an agent publishes a site that shows their available listings, without private details', async ({ page, browser }) => {
  await signIn(page, freshEmail('site'));
  await addListing(page, { 'نوع العرض': 'فيلا للبيع', 'الموقع': 'حي الملقا', 'السعر': '1500000', 'بيانات البائع': 'أبو فهد 0500000001' });
  await addListing(page, { 'نوع العرض': 'أرض للبيع', 'الموقع': 'حي حطين', 'السعر': '900000' });

  await page.getByRole('link', { name: 'موقعي' }).click();
  await expect(page.getByRole('heading', { name: 'موقعي' })).toBeVisible();
  await expectNoSidewaysScroll(page);
  const slug = `e2e-${Date.now().toString(36)}${Math.floor(Math.random() * 1e4)}`;
  await page.getByLabel('عنوان الموقع').fill(slug);
  await page.getByLabel('اسم المكتب').fill('مكتب النخبة العقاري');
  await page.getByLabel('رقم رخصة فال').fill('1200012345');
  await page.getByLabel('العبارة الرئيسية').fill('نبيع ونؤجر في شمال الرياض');
  await page.getByLabel('الخدمات').fill('تسويق العقارات\nإدارة الأملاك');
  await page.getByLabel('رقم واتساب').fill('0551234567');
  await page.getByRole('checkbox', { name: /حي حطين/ }).uncheck();
  await page.getByRole('checkbox', { name: 'نشر الموقع ليراه الجميع' }).check();
  await page.getByRole('button', { name: 'حفظ' }).click();
  await expect(page.locator('.ok')).toContainText('موقعك منشور');

  // A visitor who is not signed in.
  const visitor = await (await browser.newContext()).newPage();
  await visitor.goto(`/s/${slug}`);
  await expect(visitor.getByRole('heading', { level: 1 })).toHaveText('نبيع ونؤجر في شمال الرياض');
  await expect(visitor.getByText('إدارة الأملاك')).toBeVisible();
  await expect(visitor.getByRole('heading', { name: 'حي الملقا' })).toBeVisible();
  await expect(visitor.locator('body')).not.toContainText('حي حطين');
  await expect(visitor.locator('body')).not.toContainText('0500000001');
  await expect(visitor.getByRole('link', { name: 'راسلنا على واتساب' })).toHaveAttribute('href', 'https://wa.me/966551234567');
  await expectNoSidewaysScroll(visitor);
});
