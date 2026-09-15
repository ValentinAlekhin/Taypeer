// Complete macOS v1 scenarios. Shared primitives and the original compact
// workspace live in compact.js; this module owns desktop forms and states.
function buildDesktopV1() {
  const page = name => pages[name] || (pages[name] = Object.assign(figma.createPage(), {name}));
  const statePage = '06 · Состояния и меню';
  const qaPage = '07 · Тема и размеры';
  const flowPage = '08 · Карта сценариев';
  [statePage, qaPage, flowPage].forEach(page);
  const art = name => figma.root.children.flatMap(p => p.children).find(n => n.name === name);
  const words = n => n.findAll(c => c.type === 'TEXT');
  function replace(name, render) {
    const old = art(name), parent = old.parent, index = parent.children.indexOf(old);
    boards.splice(boards.findIndex(b => b.id === old.id), 1);
    old.remove();
    const fresh = render();
    parent.insertChild(index, fresh);
    return fresh;
  }
  function caption(p, s, x, y, w, color = 'muted', h = 40) {
    return text(p, s, x, y, w, 13, color, 400, h);
  }
  function heading(p, s, x, y, w) { return text(p, s, x, y, w, 16, 'fg', 500, 24); }
  function secret(p, value, x, y, w, copy = true) {
    const f = inputFrame(p, 'Secret / masked', x, y, w);
    text(f, value, 10, 6, w - (copy ? 88 : 48), 13);
    ib(f, 'eye', w - (copy ? 76 : 36), 0).name = 'Icon button / Раскрыть значение';
    if (copy) ib(f, 'copy', w - 36, 0).name = 'Icon button / Копировать вариант';
    return f;
  }
  function check(p, label, x, y, w, on = false) {
    const n = box(p, 'Checkbox / ' + label + ' / ' + (on ? 'checked' : 'unchecked'), x, y, w, 32, '');
    const mark = box(n, 'Checkbox mark', 0, 8, 16, 16, on ? 'selected' : 'bg', 3, true);
    if (on) icon(mark, 'check', 0, 0);
    text(n, label, 28, 6, w - 28, 13);
    return n;
  }
  function multiline(p, label, value, x, y, w, h = 76) {
    text(p, label, x, y + 6, UI.label, 13, 'muted');
    const n = inputFrame(p, label, x + 160, y, w - 160, h);
    text(n, value, 10, 8, n.width - 20, 13, 'fg', 400, h - 16);
    return n;
  }
  function fileField(p, label, value, x, y, w) {
    field(p, label, value, x, y, w - 116);
    button(p, 'Выбрать…', x + w - 104, y, 104);
  }
  function footer(p, action, {kind = 'primary', cancel = 'Отмена', width = 160} = {}) {
    const y = p.height - 64;
    rule(p, 0, y - 16, p.width);
    button(p, cancel, p.width - width - 140, y, 104);
    return button(p, action, p.width - width - 24, y, width, kind);
  }
  function modal(name, title, h = 480, w = 680) {
    const p = board('03 · Диалоги', 'Dialog / ' + name, w, h);
    heading(p, title, 24, 22, w - 100);
    ib(p, 'x', w - 56, 16).name = 'Icon button / Закрыть диалог';
    return p;
  }
  function notice(p, name, message, x, y, w, {tone = 'warning', action, h = 68} = {}) {
    const n = box(p, 'Notice / ' + name, x, y, w, h, 'panel', 4, true);
    box(n, 'State marker', 0, 0, 3, h, tone);
    caption(n, message, 16, 12, w - (action ? 204 : 32), tone, h - 24);
    if (action) button(n, action, w - 184, (h - 32) / 2, 168);
    return n;
  }
  function progress(p, label, x, y, w, fraction = 0.6) {
    caption(p, label, x, y, w, 'muted', 22);
    box(p, 'Progress track', x, y + 32, w, 3, 'border', 1);
    box(p, 'Progress value', x, y + 32, w * fraction, 3, 'muted', 1);
  }
  function table(p, name, x, y, w, columns, rows, {selected = 0, rowHeight = 48} = {}) {
    const n = box(p, 'Table / ' + name, x, y, w, 38 + rows.length * rowHeight, '');
    let xx = 16;
    columns.forEach(([label, width]) => { text(n, label, xx, 10, width - 24, 12, 'muted'); xx += width; });
    rule(n, 0, 37, w);
    rows.forEach((values, i) => {
      const yy = 38 + i * rowHeight;
      box(n, 'Row / ' + values[0], 0, yy, w, rowHeight, i === selected ? 'selected' : i % 2 ? 'panel' : 'bg');
      let dx = 16;
      values.forEach((value, j) => { text(n, value, dx, yy + 14, columns[j][1] - 24, 13, j ? 'muted' : 'fg', j ? 400 : 500, rowHeight - 20); dx += columns[j][1]; });
      rule(n, 0, yy + rowHeight - 1, w);
    });
    return n;
  }
  function sheet(name, title, subtitle, h = 1000) {
    const p = board(statePage, name, 1320, h);
    heading(p, title, 24, 24, 1240);
    caption(p, subtitle, 24, 60, 1240);
    return p;
  }
  function sample(p, title, x, y, w, h) {
    text(p, title, x, y, w, 12, 'muted', 500);
    return box(p, 'Component / ' + title, x, y + 28, w, h, 'bg', 4, true);
  }
  function menu(p, title, x, y, w, items) {
    const n = sample(p, title, x, y, w, items.length * 34 + 16);
    items.forEach((item, i) => {
      const [label, shortcut = '', state = ''] = Array.isArray(item) ? item : [item];
      const yy = 8 + i * 34;
      if (state === 'selected') box(n, 'Selected menu item', 4, yy, w - 8, 32, 'selected', 3);
      text(n, label, 12, yy + 6, w - (shortcut ? 92 : 24), 13, state === 'danger' ? 'danger' : state === 'disabled' ? 'dim' : 'fg');
      if (shortcut) text(n, shortcut, w - 84, yy + 6, 72, 12, 'muted').textAlignHorizontal = 'RIGHT';
    });
    return n;
  }
  function status(p, value, color = 'muted') {
    const n = words(p).find(n => n.characters === 'Синхронизировано');
    if (n) { n.characters = value; n.name = 'Sync state / ' + value; fill(n, color); n.x = p.width - 390; n.resize(350, 20); }
  }

  replace('Dialog / create', () => {
    const p = modal('create', 'Создать базу', 680, 720);
    field(p, 'Название', 'Рабочая', 24, 76, 672);
    multiline(p, 'Описание', 'Личные проекты и рабочие сервисы', 24, 124, 672, 64);
    field(p, 'Мастер-пароль', '••••••••••••••••', 24, 204, 672, {secret:true});
    caption(p, 'Качество: высокое', 184, 244, 460, 'success', 22);
    field(p, 'Повтор пароля', '••••••••••••••••', 24, 278, 672, {secret:true});
    caption(p, 'Целевое время\nподготовки ключа', 24, 328, 144, 'muted', 44);
    const f = inputFrame(p, 'KDF target / 0.5–5 seconds', 184, 328, 512); text(f, '1 секунда', 10, 6, 470, 13);
    caption(p, 'Бюджет калибровки: 0,5–5 с. На другом устройстве\nподготовка ключа может занять больше времени.', 184, 372, 512, 'muted', 48);
    fileField(p, 'Файл БД', '~/Documents/Рабочая.taypeer', 24, 444, 672);
    caption(p, 'Без мастер-пароля и работающей биометрии\nвосстановить доступ к данным невозможно.', 184, 500, 512, 'muted', 48);
    footer(p, 'Создать'); return p;
  });
  replace('macOS / unlock', () => {
    const p = shell('macOS / unlock');
    icon(p, 'lock-keyhole', 408, 192); heading(p, 'Рабочая.taypeer', 440, 186, 480);
    caption(p, '~/Documents/Рабочая.taypeer', 408, 226, 536, 'muted', 24);
    field(p, 'Мастер-пароль', '••••••••••••', 408, 284, 536, {secret:true});
    caption(p, 'Подготовка ключа при последнем входе: 1,2 с', 568, 328, 376, 'muted', 40);
    button(p, 'Назад', 408, 398, 104);
    button(p, 'Touch ID', 528, 398, 128, 'default', false, 'fingerprint');
    button(p, 'Разблокировать', 744, 398, 200, 'primary');
    notice(p, 'received while locked', 'Изменения получены. Разблокируйте БД,\nчтобы применить их.', 408, 470, 536);
    status(p, 'Получено · требуется вход'); return p;
  });
  replace('macOS / settings', () => {
    const p = utility('settings', 'Настройки');
    text(p, 'Устройство', 249, 100, 128, 13, 'fg', 500); text(p, 'База данных', 401, 100, 144, 13, 'muted');
    rule(p, 225, 127, 1095); box(p, 'Selected tab / Устройство', 225, 126, 152, 2, 'muted');
    field(p, 'Имя устройства', 'MacBook Pro', 249, 152, 800);
    field(p, 'Язык', 'Русский', 249, 200, 800, {select:true});
    field(p, 'Тема', 'Системная', 249, 248, 800, {select:true});
    field(p, 'Автоблокировка', 'Через 5 минут', 249, 296, 800, {select:true});
    field(p, 'Очистка буфера', 'Через 30 секунд', 249, 344, 800, {select:true});
    heading(p, 'Touch ID · Рабочая.taypeer', 249, 412, 800);
    toggle(p, 'Разблокировать эту БД на этом Mac', 249, 456, 800, true);
    heading(p, 'Соединение', 249, 516, 800);
    field(p, 'Relay', 'Общие relay', 249, 560, 800, {select:true});
    caption(p, 'Relay пересылает трафик, пока оба устройства доступны.\nЛокальная работа не требует сети.', 409, 610, 640);
    return p;
  });
  const dbSettings = utility('settings-database', 'Настройки');
  text(dbSettings, 'Устройство', 249, 100, 128, 13, 'muted'); text(dbSettings, 'База данных', 401, 100, 144, 13, 'fg', 500);
  rule(dbSettings, 225, 127, 1095); box(dbSettings, 'Selected tab / База данных', 377, 126, 176, 2, 'muted');
  heading(dbSettings, 'Рабочая.taypeer', 249, 152, 800);
  caption(dbSettings, 'MacBook Pro · это управляющее устройство', 249, 184, 800);
  field(dbSettings, 'Название', 'Рабочая', 249, 230, 800);
  multiline(dbSettings, 'Описание', 'Личные проекты и рабочие сервисы', 249, 278, 800, 64);
  button(dbSettings, 'Применить', 881, 358, 168, 'primary');
  rule(dbSettings, 249, 410, 1015);
  heading(dbSettings, 'Защита и общие лимиты', 249, 434, 850);
  button(dbSettings, 'Изменить мастер-пароль…', 249, 478, 256);
  button(dbSettings, 'Параметры защиты…', 521, 478, 224);
  field(dbSettings, 'Одно вложение', '10 МиБ', 249, 536, 800);
  field(dbSettings, 'Все вложения', '100 МиБ', 249, 584, 800);
  caption(dbSettings, 'Максимум: 100 МиБ на файл и 1 ГиБ на БД.', 409, 626, 640, 'muted', 24);
  button(dbSettings, 'Применить лимиты…', 825, 670, 224, 'primary');
  button(dbSettings, 'Обновить формат…', 249, 728, 224);
  const publicSettings = shell('macOS / settings-locked');
  // No database contents are available behind pre-unlock settings.
  const headerTexts = words(publicSettings).filter(n => n.characters === 'Рабочая.taypeer');
  headerTexts.forEach(n => {n.characters = 'Taypeer';});
  publicSettings.children.filter(n => n.name === 'StatusBar').forEach(n => n.remove());
  publicSettings.children.filter(n => n.y >= 790).forEach(n => n.remove());
  heading(publicSettings, 'Настройки устройства', 352, 168, 640);
  field(publicSettings, 'Язык', 'Русский', 352, 230, 616, {select:true});
  field(publicSettings, 'Тема', 'Системная', 352, 278, 616, {select:true});
  caption(publicSettings, 'Параметры БД доступны после разблокировки.', 512, 342, 456);
  button(publicSettings, 'Готово', 808, 408, 160);

  buildLocalScenarios();
  buildTrustScenarios();
  buildTransferScenarios();
  buildComponentSheets();
  buildReviewVariants();
  buildFlowMap();
  arrangePages();

  // Scenario builders follow; their local helpers stay inside this module.
  function duplicate(source, name, destination) {
    const n = art(source).clone();
    page(destination).appendChild(n); n.name = name;
    boards.push({id:n.id, name, page:destination});
    return n;
  }
  function confirm(name, title, summary, action, {danger = false, detail = '', h = 330} = {}) {
    const p = modal(name, title, h);
    caption(p, summary, 24, 80, p.width - 48, 'fg', 64);
    if (detail) caption(p, detail, 24, 156, p.width - 48, 'muted', 76);
    footer(p, action, {kind:danger ? 'danger' : 'primary', width:208});
    return p;
  }
  function buildLocalScenarios() {
    const fresh = duplicate('macOS / edit', 'macOS / new-entry', '01 · macOS');
    const freshEditor=fresh.children.find(n=>n.name==='Editor');
    words(freshEditor).filter(n => n.characters === 'GitHub').forEach(n => {n.characters = n.parent.name === 'Editor' ? 'Новая запись' : '';});
    fresh.findAll(n => n.name.startsWith('Input / row /')).forEach(n => {
      n.children.filter(c => c.type === 'TEXT').forEach(c => {c.characters = n.name.includes('Срок действия') ? 'Бессрочно' : '';});
    });
    const group = modal('group', 'Создать группу', 430);
    field(group, 'Название', 'Сервисы', 24, 80, 632);
    multiline(group, 'Описание', 'Сервисы для работы', 24, 128, 632, 72);
    field(group, 'Родитель', 'Работа', 24, 216, 632, {select:true});
    text(group, 'Значок', 24, 270, 144, 13, 'muted');
    button(group, 'Выбрать значок…', 184, 264, 216, 'default', false, 'folder');
    button(group, 'Загрузить по URL…', 416, 264, 240);
    footer(group, 'Подтвердить');
    const move = modal('move-entry', 'Переместить запись', 476);
    caption(move, 'GitHub · Рабочая / Работа', 24, 72, 632);
    table(move, 'destination', 24, 124, 632, [['Группа назначения',632]], [['Работа'],['    Сервисы'],['    Инфраструктура'],['Личное']], {selected:3});
    footer(move, 'Переместить');
    const moveGroup = modal('move-group','Переместить группу',442);
    caption(moveGroup,'Сервисы · перемещение всего поддерева',24,78,632,'fg');
    field(moveGroup,'Родитель','Работа',24,132,632,{select:true});
    field(moveGroup,'Положение','Перед «Инфраструктура»',24,186,632,{select:true});
    caption(moveGroup,'Можно выбрать верхний уровень. Перемещение внутрь\nсвоего поддерева недоступно.',24,254,632,'muted',56);
    footer(moveGroup,'Переместить');
    confirm('delete-objects', 'Переместить группу в корзину?', 'Работа · 3 группы и 6 записей', 'В корзину', {danger:true, detail:'Поддерево останется доступно в корзине.\nЕго можно восстановить до окончательной очистки.'});

    const trash = utility('trash', 'Корзина');
    button(trash, 'Очистить корзину…', 1056, 51, 232, 'danger');
    table(trash, 'trash', 249, 118, 1039, [['Название',310],['Исходная группа',310],['Удалено',230],['Состояние',189]], [
      ['Старые проекты','Рабочая','Сегодня, 10:15','Группа · 4 записи'],
      ['GitHub','Работа / Сервисы','Вчера, 18:32','Конфликт'],
      ['Wi-Fi','Личное','12 сентября','Запись']
    ]);
    heading(trash, 'Старые проекты', 249, 352, 960);
    caption(trash, 'Группа и её содержимое будут восстановлены вместе.', 249, 394, 960);
    button(trash, 'Восстановить…', 249, 452, 208, 'primary');
    button(trash, 'Удалить навсегда…', 473, 452, 216, 'danger');
    caption(trash, 'Корзина хранится до ручной очистки.', 249, 720, 960);
    confirm('purge-trash', 'Очистить корзину?', 'Будут удалены 1 группа и 6 записей.', 'Очистить корзину', {danger:true, detail:'Данные перестанут быть доступны в рабочей БД.\nВ старых файлах, резервных копиях и на других устройствах\nмогут сохраниться прежние данные.', h:370});
    const destination = modal('restore-destination', 'Восстановить запись', 450);
    caption(destination, 'GitHub · исходная группа «Сервисы» отсутствует.', 24, 76, 632, 'fg');
    field(destination, 'Группа', 'Выберите группу', 24, 144, 632, {select:true});
    button(destination, 'Создать группу…', 184, 202, 224);
    notice(destination, 'destination required', 'Для восстановления выберите существующую\nгруппу или создайте новую.', 24, 260, 632);
    footer(destination, 'Восстановить', {kind:'disabled'});

    const comparison = utility('history-compare', 'История · GitHub');
    button(comparison, 'К записи', 1156, 51, 132);
    caption(comparison, '6 сентября, 18:32 · MacBook Pro', 249, 120, 1039);
    table(comparison, 'revision comparison', 249, 168, 1039, [['Поле',184],['Выбранная версия',424],['Текущая версия',431]], [
      ['Название','GitHub','GitHub'],['Логин','valentin','valentin.dev'],
      ['URL','https://github.com','https://github.com'],['Теги','работа','работа, разработка']
    ], {selected:-1});
    text(comparison, 'Пароль', 265, 430, 160, 13, 'muted');
    secret(comparison, '••••••••••••', 449, 424, 388);
    secret(comparison, '••••••••••••••••', 873, 424, 391);
    text(comparison, 'Атрибуты', 265, 490, 160, 13, 'muted');
    secret(comparison, 'Recovery code · ••••••••', 449, 484, 388);
    secret(comparison, 'Recovery code · ••••••••', 873, 484, 391);
    caption(comparison, 'Вложения', 265, 550, 160);
    caption(comparison, 'recovery-codes.txt · 12 КиБ', 449, 550, 388);
    caption(comparison, 'recovery-codes.txt · 20 КиБ', 873, 550, 391);
    button(comparison, 'Просмотреть версию…', 249, 620, 256);
    button(comparison, 'Восстановить версию…', 1008, 700, 280, 'primary');
    confirm('restore-revision', 'Восстановить версию GitHub?', '6 сентября, 18:32 · MacBook Pro', 'Восстановить', {detail:'Выбранные данные станут новой текущей версией.\nПрежние сохранённые версии останутся в истории.'});
    confirm('clear-history', 'Очистить историю GitHub?', 'Будут очищены 3 сохранённые версии.', 'Очистить историю', {danger:true, detail:'Текущая запись сохранится. Неразрешённые варианты\nконфликтов остаются до их разбора. Старые резервные\nкопии этой операцией не изменяются.', h:370});
    const revision=art('macOS / main').children.find(n=>n.name==='Entry detail').clone();
    page('05 · Вкладки записи').appendChild(revision);revision.name='Tabs / saved-revision';revision.x=0;revision.y=0;revision.resize(614,650);
    boards.push({id:revision.id,name:revision.name,page:'05 · Вкладки записи'});
    revision.findAll(n=>n.name==='Icon button / Редактировать запись').forEach(n=>n.remove());
    words(revision).filter(n=>n.characters==='GitHub'&&n.y<44).forEach(n=>{n.characters='GitHub · 6 сентября';n.resize(470,26);});
    caption(revision,'Сохранённая версия · MacBook Pro · 18:32',24,436,566);
    button(revision,'К истории',24,560,176);
    button(revision,'Восстановить версию…',296,560,294,'primary');

    const attribute = modal('attribute', 'Добавить атрибут', 358);
    field(attribute, 'Ключ', 'Recovery code', 24, 80, 632);
    field(attribute, 'Значение', '••••••••••••', 24, 128, 632, {secret:true});
    check(attribute, 'Защищённое значение', 184, 176, 448, true);
    caption(attribute, 'Скрывается при просмотре и исключается из поиска.', 184, 224, 448);
    footer(attribute, 'Добавить');
    const rename = modal('attachment-rename', 'Переименовать вложение', 250);
    field(rename, 'Имя файла', 'recovery-codes-2026.txt', 24, 90, 632);
    footer(rename, 'Переименовать');
    const pick = modal('icon-picker', 'Выбрать значок', 500);
    const search = inputFrame(pick, 'Icon search', 24, 76, 632); icon(search, 'search', 10, 8); text(search, 'Поиск Lucide', 38, 6, 576, 13, 'muted');
    const iconNames = Object.keys(icons);
    iconNames.forEach((name,i) => {
      const x = 24 + i % 10 * 62, y = 136 + Math.floor(i / 10) * 54;
      box(pick, 'Icon option / ' + name, x, y, 48, 44, name === 'key-round' ? 'selected' : 'panel', 4);
      icon(pick, name, x + 16, y + 14, 'fg');
    });
    caption(pick, 'Выбран: Lucide · key-round', 24, 368, 632);
    footer(pick, 'Выбрать');
    const color = modal('color-picker', 'Цвет текста записи', 380);
    ['#E6E6E6','#EE9292','#D0B57D','#8BB694','#9FBFE8','#BFA6DA','#DDA6C0','#747474'].forEach((c,i) => box(color, 'Color option / '+c, 24+i*78, 88, 62, 48, c, 4));
    field(color, 'Цвет RGBA', '#E6E6E6FF', 24, 172, 632);
    button(color, 'По умолчанию', 184, 226, 208);
    footer(color, 'Выбрать');
    const imageUrl=modal('image-url','Загрузить значок по URL',366);
    field(imageUrl,'URL изображения','https://example.test/icon.png',24,82,632);
    caption(imageUrl,'Изображение до 1 МиБ сохранится в БД.\nЗапрос отправляется только по нажатию «Загрузить».',24,144,632,'muted',60);
    footer(imageUrl,'Загрузить');
    const bulk = modal('bulk-icons', 'Загрузить значки группы', 490);
    caption(bulk, 'Работа · 6 записей', 24, 78, 632, 'fg');
    check(bulk, 'Включить вложенные группы', 24, 126, 632);
    check(bulk, 'Заменить уже заданные значки', 24, 174, 632);
    caption(bulk, 'Запросы будут отправлены на сайты из URL записей.', 24, 234, 632);
    notice(bulk, 'partial favicon result', 'Загружено: 4. Пропущено: 1. Ошибка: 1.\nУспешно загруженные значки сохранены.', 24, 298, 632, {action:'Ошибки…', h:76});
    footer(bulk, 'Загрузить', {width:160});
  }
  function buildTrustScenarios() {
    replace('macOS / devices', () => {
      const p = utility('devices', 'Устройства');
      button(p, 'Поделиться…', 1100, 51, 188, 'default', false, 'plus');
      table(p, 'devices', 249, 122, 1039, [['Устройство',280],['Доступность',200],['Роль',220],['Обмен',339]], [
        ['MacBook Pro','Это устройство','Управляющее','Изменения применены'],
        ['Pixel 9','В сети','Доверенное','Получено · ждёт входа'],
        ['Mac mini','Не в сети · 2 ч','Доверенное','Ожидание устройства']
      ],{selected:1});
      button(p, 'Действия устройства…', 249, 344, 252);
      heading(p, 'Синхронизация', 249, 422, 960);
      caption(p, 'Pixel 9 · прямое соединение', 249, 466, 960);
      notice(p, 'receive and apply', 'Изменения получены на Pixel 9.\nДля применения разблокируйте БД на этом устройстве.', 249, 512, 1039, {h:76});
      button(p, 'Синхронизировать', 1064, 628, 224, 'default', false, 'refresh-cw');
      status(p, 'Получено · ожидается применение');
      return p;
    });
    replace('Dialog / invite', () => {
      const p = modal('invite', 'Поделиться БД · Рабочая', 650, 744);
      const qr = box(p, 'QR / synthetic invitation', 24, 88, 232, 232, '#FFFFFF');
      const modules = DEMO_INVITATION.modules, unit = 232 / (modules.length + 8);
      const v = figma.createVector(); qr.appendChild(v); v.name = 'QR modules / same data as invitation code';
      const paths = [];
      modules.forEach((row,y) => [...row].forEach((bit,x) => {
        if (bit === '1') { const a=(x+4)*unit,b=(y+4)*unit; paths.push(`M${a} ${b}L${a+unit} ${b}L${a+unit} ${b+unit}L${a} ${b+unit}Z`); }
      }));
      v.vectorPaths = [{windingRule:'NONZERO',data:paths.join('')}]; v.fills=paint('#000000'); v.strokes=[];
      // The path setter normalizes coordinates and can subtract the parent's
      // canvas position. Place the normalized bounds at the intended quiet zone.
      v.x=4*unit; v.y=4*unit;
      text(p, 'Код приглашения', 288, 88, 408, 12, 'muted');
      text(p, DEMO_INVITATION.code.match(/.{1,24}/g).join('\n'), 288, 122, 408, 14, 'fg', 400, 104);
      button(p, 'Копировать код', 288, 244, 216, 'default', false, 'copy');
      caption(p, 'Одноразовое · осталось 04:32', 288, 292, 408);
      rule(p, 24, 352, 696);
      heading(p, 'Pixel 9 запрашивает доступ', 24, 384, 696);
      caption(p, 'Подтвердите подключение своего устройства.\nДля чтения БД на нём потребуется мастер-пароль.', 24, 430, 696, 'muted', 56);
      footer(p, 'Разрешить', {cancel:'Отклонить'}); return p;
    });
    const receive = shell('macOS / receive');
    heading(receive, 'Получить базу данных', 352, 152, 640);
    caption(receive, 'Приглашение', 352, 200, 640);
    multiline(receive, 'Код', DEMO_INVITATION.code.match(/.{1,24}/g).join('\n'), 352, 240, 640, 108);
    button(receive, 'Сканировать QR…', 512, 368, 224);
    caption(receive, 'Управляющее устройство должно быть в сети\nи подтвердить подключение этого Mac.', 512, 420, 480, 'muted', 56);
    button(receive, 'Назад', 352, 520, 128);
    button(receive, 'Подключиться', 792, 520, 200, 'primary');
    const scanner = modal('scan-qr', 'Сканировать приглашение', 464, 600);
    const camera = box(scanner, 'Camera preview', 24, 76, 552, 248, 'panel', 4, true);
    box(camera, 'QR capture area', 188, 38, 176, 176, '', 4, true);
    caption(camera, 'Наведите камеру на QR', 96, 216, 360, 'muted', 24);
    caption(scanner, 'Доступ к камере запрашивается средствами macOS.', 24, 346, 552);
    footer(scanner, 'Ввести код', {kind:'default', width:176});

    function passwordOperation(name, title, action, revoke = false) {
      const p = modal(name, title, 500, 744);
      caption(p, revoke ? 'Pixel 9 будет исключён из доверенных устройств.' : 'Рабочая.taypeer · новое состояние защиты', 24, 78, 696, 'fg');
      field(p, 'Новый пароль', '••••••••••••••••', 24, 140, 696, {secret:true});
      caption(p, 'Качество: высокое', 184, 180, 512, 'success', 24);
      field(p, 'Повтор пароля', '••••••••••••••••', 24, 220, 696, {secret:true});
      caption(p, 'Будет сохранена отдельная исходная копия. Другим\nустройствам понадобятся новый пароль и настройка Touch ID\nили биометрии заново после получения изменений.', 24, 280, 696, 'muted', 72);
      if (revoke) caption(p, 'Ранее полученные данные останутся на Pixel 9.', 24, 368, 696, 'warning', 28);
      footer(p, action, {kind:revoke ? 'danger' : 'primary', width:216}); return p;
    }
    replace('Dialog / revoke', () => passwordOperation('revoke','Отозвать доступ Pixel 9?','Отозвать доступ',true));
    passwordOperation('change-password','Изменить мастер-пароль','Изменить пароль');
    const protection = modal('protection', 'Параметры защиты БД', 390, 744);
    caption(protection, 'Целевое время\nподготовки ключа', 24, 84, 144, 'muted', 44);
    const kdf = inputFrame(protection, 'KDF target', 184, 84, 536); text(kdf, '1 секунда', 10, 6, 510, 13);
    caption(protection, 'Допустимо 0,5–5 секунд. Калибровка выполняется\nна этом управляющем устройстве; на других устройствах\nфактическое время может отличаться.', 184, 142, 536, 'muted', 72);
    caption(protection, 'Последнее фактическое время: 1,2 с', 184, 236, 536);
    footer(protection, 'Применить', {width:184});
    const handoff = modal('transfer-control', 'Передать управление БД', 430, 744);
    field(handoff, 'Устройство', 'Pixel 9 · в сети', 24, 84, 696, {select:true});
    notice(handoff, 'handoff prerequisites', 'Оба устройства должны быть подключены и разблокированы.\nПередачу нужно подтвердить на Pixel 9.', 24, 148, 696, {h:76});
    caption(handoff, 'После передачи этот Mac останется доверенным устройством.\nПароль, допуски и общие параметры будет менять Pixel 9.', 24, 252, 696, 'muted', 56);
    footer(handoff, 'Передать управление', {width:240});
    confirm('accept-control', 'Принять управление БД?', 'Рабочая.taypeer · запрос от MacBook Pro', 'Принять управление', {detail:'Этот Mac станет единственным управляющим устройством.\nПодтверждение завершит начатую передачу.', h:340});
    const recover = modal('recover-control', 'Восстановить управление', 584, 744);
    caption(recover, 'Рабочая.taypeer · исходный файл сохранится', 24, 78, 696, 'fg');
    fileField(recover, 'Новый файл', '~/Documents/Рабочая-new.taypeer', 24, 130, 696);
    field(recover, 'Новый пароль', '••••••••••••••••', 24, 190, 696, {secret:true});
    field(recover, 'Повтор пароля', '••••••••••••••••', 24, 238, 696, {secret:true});
    notice(recover, 'new trust set', 'Этот Mac станет управляющим нового доверенного набора.\nОстальные устройства потребуется подключить заново.', 24, 310, 696, {h:80});
    caption(recover, 'Прежние допуски не переносятся. Старые копии сохранят\nпрежние данные и пароль.', 24, 412, 696, 'muted', 52);
    footer(recover, 'Восстановить', {width:216});

    replace('macOS / conflict', () => {
      const p = utility('conflict', 'Конфликты · 3');
      table(p, 'conflict list', 249, 122, 328, [['Объект / конфликт',328]], [['GitHub · пароль'],['Wi-Fi · удаление'],['Работа · размещение']]);
      rule(p, 601, 90, 1, 702);
      heading(p, 'GitHub / Пароль', 626, 124, 636);
      radio(p, 'MacBook Pro · сегодня, 09:32', 626, 180, 636, false);
      secret(p, '••••••••••••••••', 654, 224, 610);
      radio(p, 'Pixel 9 · сегодня, 09:35', 626, 288, 636, false);
      secret(p, '••••••••••••', 654, 332, 610);
      radio(p, 'Своё значение', 626, 396, 636, false);
      field(p, 'Новый пароль', '', 626, 448, 638, {secret:true});
      caption(p, 'Выберите вариант или введите свой.\nИсходные варианты останутся в истории.', 626, 526, 638, 'muted', 52);
      button(p, 'Применить', 1096, 700, 168, 'disabled').name='Button / disabled / Resolve conflict';
      status(p, 'Применено · 3 конфликта', 'warning'); return p;
    });
    const pending = utility('pending', 'Отложенные правки · 4');
    table(pending, 'pending edits', 249, 116, 1039, [['Источник',250],['Объект',270],['Причина',519]], [
      ['Pixel 9 · 10:24','GitHub','Автор больше не доверенный'],
      ['Mac mini · вчера','Старый проект','Объект окончательно очищен'],
      ['Mac mini · 12 сентября','3 записи','Ожидает управляющее устройство'],
      ['Pixel 9 · 10:20','Recovery codes','Ожидается содержимое вложений']
    ]);
    heading(pending, 'GitHub · данные от Pixel 9', 249, 388, 1039);
    caption(pending, 'Источник сохранён отдельно. Его просмотр не меняет текущую БД.', 249, 428, 1039);
    text(pending,'Логин',249,484,144,13,'muted');text(pending,'valentin.dev',409,484,640,13);
    rule(pending,249,520,800);
    text(pending, 'Пароль', 249, 534, 144, 13, 'muted'); secret(pending, '••••••••••••', 409, 528, 640);
    field(pending, 'Группа назначения', 'Работа / Сервисы', 249, 584, 800, {select:true});
    button(pending, 'Извлечь запись…', 249, 666, 224, 'primary');
    button(pending, 'Удалить источник…', 489, 666, 240, 'danger');
    caption(pending, 'Остальные допустимые изменения применяются независимо.', 249, 726, 1000);
    status(pending, 'Получено · 4 отложенных источника', 'warning');
    confirm('extract-pending', 'Извлечь выбранную запись?', 'GitHub → Работа / Сервисы', 'Извлечь запись', {detail:'Будет создана новая запись от этого устройства\nс сохранением происхождения. Сетевой допуск Pixel 9\nне восстанавливается.', h:370});
    confirm('delete-pending', 'Удалить отложенный источник?', 'Pixel 9 · GitHub · сегодня, 10:24', 'Удалить источник', {danger:true, detail:'Непринятые данные этого источника станут недоступны.\nУже принятые изменения и другие источники сохранятся.', h:340});
  }
  function buildTransferScenarios() {
    const gen = art('Dialog / generator');
    words(gen).forEach(n => {
      if(n.characters === '24') n.characters = '30';
      if(n.characters === '24 символа') n.characters = '30 символов';
      if(n.characters === 'Энтропия: 157,3 бит') n.characters = 'Энтропия: 196,64 бит';
      if(n.characters.startsWith('mT7!')) n.characters = 'mT7!qR4#vN9@kL2$wH6%pD8&xC3?zB';
    });
    const phrase = board('03 · Диалоги', 'Dialog / passphrase', 744, 438);
    const result = inputFrame(phrase, 'Generated passphrase', 24, 24, 608, 40);
    const phraseText = 'river-candle-paper-meadow-copper-sunset';
    text(result, phraseText, 12, 10, 540, 14); ib(result,'eye',572,4);
    ib(phrase,'dice-5',640,28).name='Icon button / Сгенерировать фразу'; ib(phrase,'copy',688,28);
    box(phrase,'Phrase quality',24,76,696,3,'success');
    text(phrase,'Качество: высокое',24,92,220,12,'muted');
    text(phrase,phraseText.length+' символов',258,92,220,12,'muted').textAlignHorizontal='CENTER';
    text(phrase,'Энтропия: 77,55 бит',500,92,220,12,'muted').textAlignHorizontal='RIGHT';
    text(phrase,'Пароль',32,144,104,13,'muted'); text(phrase,'Парольная фраза',152,144,264,13,'fg',500);
    rule(phrase,24,176,696); box(phrase,'Selected tab / Парольная фраза',144,175,180,2,'muted');
    field(phrase,'Количество слов','6',24,202,696);
    field(phrase,'Разделитель','-',24,250,696);
    caption(phrase,'EFF Long Wordlist · английский · работает без сети',184,300,536);
    footer(phrase,'Использовать',{cancel:'Закрыть',width:184});
    const exclusions = modal('generator-exclusions','Параметры генератора',392);
    check(exclusions,'Исключить похожие символы',24,80,632,true);
    field(exclusions,'Исключить символы','O0Il1',24,140,632);
    caption(exclusions,'Длина пароля: 1–256 символов. Выбранные наборы\nобразуют общий алфавит после исключений.',24,204,632,'muted',56);
    footer(exclusions,'Готово');

    replace('macOS / transfer', () => {
      const p = utility('transfer','Импорт и экспорт');
      heading(p,'Импорт KDBX',249,130,1039);
      caption(p,'Будет создана отдельная БД Taypeer. Исходный файл сохранится.',249,170,1039);
      fileField(p,'Файл KDBX','~/Downloads/Личная.kdbx',249,224,864);
      field(p,'Способ открытия','Пароль и ключевой файл',249,272,864,{select:true});
      field(p,'Пароль KDBX','••••••••••••',249,320,864,{secret:true});
      fileField(p,'Ключевой файл','~/Documents/Личная.keyx',249,368,864);
      button(p,'Продолжить…',913,428,200,'primary');
      rule(p,249,490,1039);
      heading(p,'Экспорт KDBX 4.1',249,520,1039);
      caption(p,'Рабочая.taypeer · 9 записей · история и корзина',249,560,1039);
      button(p,'Экспортировать…',249,622,224);
      return p;
    });
    const importReview = modal('import-review','Импортировать в новую БД',700,792);
    field(importReview,'Название','Личная',24,76,744);
    fileField(importReview,'Новый файл','~/Documents/Личная.taypeer',24,124,744);
    field(importReview,'Мастер-пароль','••••••••••••••••',24,172,744,{secret:true});
    field(importReview,'Повтор пароля','••••••••••••••••',24,220,744,{secret:true});
    caption(importReview,'Качество: высокое',184,262,584,'success',24);
    heading(importReview,'Отчёт до импорта',24,314,744);
    table(importReview,'import report',24,350,744,[['Данные',480],['Перенос',264]],[
      ['Группы и записи','4 группы · 26 записей'],['Атрибуты, вложения, история','Поддерживаются'],['Иконки и оформление','2 несовпадения']
    ],{selected:-1,rowHeight:42});
    caption(importReview,'Отдельные настройки и плагины KeePass не переносятся.',24,534,744,'warning',30);
    button(importReview,'Подробный отчёт…',24,574,240);
    footer(importReview,'Импортировать',{width:200});
    const exportForm = modal('export-kdbx','Экспортировать KDBX 4.1',440,744);
    caption(exportForm,'Рабочая.taypeer · экспорт в отдельный файл',24,78,696,'fg');
    fileField(exportForm,'Файл KDBX','~/Documents/Рабочая.kdbx',24,130,696);
    field(exportForm,'Пароль KDBX','••••••••••••••••',24,190,696,{secret:true});
    field(exportForm,'Повтор пароля','••••••••••••••••',24,238,696,{secret:true});
    caption(exportForm,'Этот пароль защищает экспортируемый файл.',184,290,536);
    footer(exportForm,'Посмотреть отчёт…',{width:224});
    const report = modal('transfer-report','Отчёт переноса',618,792);
    table(report,'loss report',24,80,744,[['Данные',392],['Результат',352]],[
      ['Группы и поля записи','Переносятся'],['Атрибуты и их защита','Переносятся'],['Вложения, история, корзина','Переносятся'],
      ['Lucide и источник иконки','Идентичность не переносится'],['Журнал объединения','Не переносится'],['Доверенные устройства и ключи','Не экспортируются']
    ],{selected:-1,rowHeight:44});
    notice(report,'loss acknowledgement','Полученный файл содержит переносимые данные.\nПолный обратимый перенос метаданных не гарантируется.',24,418,744,{h:76});
    footer(report,'Создать KDBX',{width:216});
    replace('macOS / backups', () => {
      const p = utility('backups','Резервные копии');
      button(p,'Сохранить копию…',1064,51,224);
      table(p,'backups',249,120,1039,[['Состояние',340],['Причина',420],['Размер',279]],[
        ['Сегодня, 09:40','Автоматическая','2,4 МиБ'],['Вчера, 18:32','Перед сменой пароля','2,4 МиБ'],['5 сентября, 12:10','Автоматическая','2,3 МиБ']
      ]);
      button(p,'Восстановить выбранную…',249,348,296,'primary');
      button(p,'Выбрать другой снимок…',561,348,288);
      caption(p,'Хранятся 10 предыдущих исправных автоматических снимков.\nОтдельные копии перед важными операциями сохраняются дополнительно.',249,418,1039,'muted',56);
      notice(p,'old backups','У старых снимков остаются прежние данные и действовавший\nдля них пароль. Копия на этом Mac не заменяет копию вне устройства.',249,512,1039,{h:84});
      return p;
    });
    const restore = modal('restore-backup','Восстановить данные снимка',622,792);
    fileField(restore,'Снимок','~/Backups/Рабочая-09-12.taypeer',24,78,744);
    field(restore,'Пароль снимка','••••••••••••',24,134,744,{secret:true});
    heading(restore,'Выбранное состояние',24,202,744);
    table(restore,'snapshot preview',24,242,744,[['Источник',392],['Текущая БД',352]],[
      ['12 сентября, 18:32','Рабочая.taypeer'],['4 группы · 8 записей','4 группы · 9 записей'],['Сохранённые данные снимка','Новые изменения в текущей БД']
    ],{selected:-1,rowHeight:44});
    caption(restore,'Сначала будет сохранена копия текущего состояния.\nАктуальные пароль, формат и управление сохранятся.',24,440,744,'muted',60);
    footer(restore,'Восстановить',{width:208});
    const migration = modal('migrate','Обновить формат БД',464,744);
    caption(migration,'Рабочая.taypeer · управляющее устройство MacBook Pro',24,78,696,'fg');
    notice(migration,'migration backup','Перед обновлением будет сохранена отдельная исходная копия.\nВосстановить её данные можно в актуальную БД.',24,134,696,{h:80});
    caption(migration,'Старым приложениям может потребоваться обновление.\nИх запоздалые правки сохранятся для переноса или разбора.\nВозврата рабочей БД к старому формату нет.',24,244,696,'muted',80);
    footer(migration,'Обновить формат',{width:216});
  }
  function buildComponentSheets() {
    const nav = sheet('Menus / navigation','Навигация и команды БД','Фрагменты раскрытых меню. Переходы и условия доступны в карте сценариев.',1100);
    menu(nav,'Селектор открытых БД',24,112,392,[['Рабочая.taypeer','', 'selected'],'Личная.taypeer · заблокирована','Открыть БД…','Создать БД…','Получить БД…','Заблокировать БД','Закрыть БД']);
    menu(nav,'Меню текущей БД',448,112,392,['Поделиться…','Устройства','Конфликты · 3','Отложенные правки · 4','Корзина','Резервные копии','Сохранить копию…','Восстановить снимок…','Импорт KDBX…','Экспорт KDBX…','Восстановить управление…']);
    menu(nav,'Область поиска',872,112,424,[['Эта база','', 'selected'],'Все разблокированные базы']);
    menu(nav,'Видимость колонок',872,300,424,['✓ Название','✓ Логин','URL','Заметки','Изменение','✓ База / группа']);
    const searchResult = sample(nav,'Переход из общего поиска',24,480,392,158);
    caption(searchResult,'GitLab · Личная / Сервисы',16,18,360,'fg');
    button(searchResult,'К результатам поиска',16,76,280,'default',false,'arrow-left');
    const groupEmpty = sample(nav,'Пустая выбранная группа',24,714,392,134);
    caption(groupEmpty,'В этой группе пока нет записей.',16,18,360);
    button(groupEmpty,'Создать запись…',16,70,224,'default',false,'file-plus-2');
    const noResults = sample(nav,'Нет результатов',448,640,392,142);
    caption(noResults,'По запросу «invoice» ничего не найдено.',16,18,360);
    button(noResults,'Очистить поиск',16,76,216);
    menu(nav,'Последняя БД / файл недоступен',872,632,424,['Выбрать файл снова…','Отмена']);
    menu(nav,'Импорт / способ открытия',448,860,392,['Только пароль','Только ключевой файл',['Пароль и ключевой файл','','selected']]);

    const actions = sheet('Menus / objects','Действия над объектами','Все команды доступны без наведения; деструктивные операции требуют точного подтверждения.',1120);
    menu(actions,'Группа',24,112,392,['Создать группу…','Изменить группу…','Клонировать группу…','Переместить группу…',['Удалить группу…','','danger'],'Загрузить значки…']);
    menu(actions,'Запись',448,112,392,['Редактировать запись','Клонировать запись…','Переместить в группу…',['Удалить запись…','','danger'],'Копировать логин','Копировать пароль']);
    menu(actions,'Вложение / режим правок',872,112,424,['Сохранить наружу…','Переименовать…','Заменить файлом…',['Удалить вложение','','danger']]);
    menu(actions,'Пустое место дерева',24,390,392,['Создать группу…']);
    menu(actions,'Атрибут / режим правок',448,424,392,['Показать значение','Копировать значение',['Удалить атрибут','','danger']]);
    menu(actions,'История',872,350,424,['Просмотреть версию…','Сравнить с текущей…','Восстановить версию…',['Очистить историю…','','danger']]);
    menu(actions,'Доверенное устройство',24,570,392,['Синхронизировать','Передать управление…',['Отозвать доступ…','','danger']]);
    const drag = sample(actions,'Перемещение группы',448,650,392,216);
    ['Работа','    Сервисы','    Инфраструктура','Личное'].forEach((s,i)=>{
      text(drag,s,28,20+i*40,330,13); icon(drag,'folder',8,22+i*40);
    });
    box(drag,'Drop indicator / sibling order',24,92,344,2,'fg');
    const nesting=sample(actions,'Перемещение внутрь группы',872,650,424,180);
    box(nesting,'Drop target',12,14,400,40,'selected',4,true); icon(nesting,'folder',24,26); text(nesting,'Работа',52,24,352,13);
    caption(nesting,'Сервисы → Работа',24,86,376);
    caption(actions,'Создание и изменение группы используют Dialog / group. Клонирование сохраняет родителя исходной группы;\nизменяемое имя задаётся в той же форме. Клонирование записи открывает новый черновик в её группе.',24,984,1272,'muted',64);

    const session = sheet('States / session','Вход, черновики и надёжное сохранение','Варианты общих компонентов: причина, доступное действие и сохранённый ввод.',1300);
    const login = sample(session,'Неверный пароль / Touch ID недоступен',24,112,616,218);
    field(login,'Мастер-пароль','••••••••••••',16,24,584,{secret:true,error:true});
    caption(login,'Не удалось разблокировать БД. Проверьте пароль.',176,70,408,'danger',40);
    caption(login,'Touch ID недоступен. Введите мастер-пароль.',16,124,584);
    button(login,'Разблокировать',360,170,240,'primary');
    const rotation = sample(session,'Вход после изменения защиты',672,112,624,218);
    notice(rotation,'new password','Защита БД изменилась. Введите новый мастер-пароль.\nБиометрию потребуется включить заново.',16,20,592,{h:76});
    field(rotation,'Новый пароль','',16,116,592,{secret:true});
    button(rotation,'Разблокировать',368,168,240,'primary');
    const draft = sample(session,'После разблокировки / сохранённый черновик',24,394,616,206);
    heading(draft,'Продолжить изменения GitHub?',16,20,584);
    caption(draft,'Правки сохранены только на этом Mac.',16,66,584);
    button(draft,'Удалить черновик',16,144,208,'danger');
    button(draft,'Продолжить',392,144,208,'primary');
    const draftError = sample(session,'Ошибка записи черновика / БД уже заблокирована',672,394,624,206);
    notice(draftError,'draft lost','Не удалось сохранить черновик: недостаточно места.\nБД заблокирована. Восстановление этих правок недоступно.',16,20,592,{tone:'danger',h:88});
    button(draftError,'Понятно',432,144,176);
    const saving = sample(session,'Создание / подготовка ключа',24,676,616,202);
    progress(saving,'Подготовка ключа и создание БД…',16,24,584,0.35);
    button(saving,'Отмена',264,138,144,'disabled');
    button(saving,'Создание…',424,138,176,'disabled');
    const saveError = sample(session,'Ошибка подтверждения записи',672,676,624,202);
    notice(saveError,'write failure','Не удалось сохранить: нет доступа к файлу.\nИзменения остались в форме.',16,20,592,{tone:'danger',h:80});
    button(saveError,'Вернуться',208,138,176); button(saveError,'Повторить',400,138,208,'primary');
    notice(session,'missing file','Файл ~/Documents/Рабочая.taypeer не найден.',24,966,616,{action:'Выбрать снова…'});
    notice(session,'corrupt file','Файл повреждён. Исходный файл не изменён.',672,966,624,{tone:'danger',action:'Выбрать снимок…'});
    notice(session,'biometric setup','Touch ID настраивается только для этой БД на этом Mac.',24,1070,616,{action:'Включить…',h:84});
    notice(session,'lost password','Без мастер-пароля и работающей биометрии\nвосстановить доступ невозможно.',672,1070,624,{h:84});

    const exchange = sheet('States / exchange','Получение БД и обмен','Получение подтверждает сохранение зашифрованных данных. Применение доступно после входа.',1280);
    const waiting=sample(exchange,'Получение БД / ожидание подтверждения',24,112,616,170);
    heading(waiting,'Ожидается подтверждение',16,20,584);
    caption(waiting,'Разрешите подключение этого Mac\nна управляющем устройстве.',16,58,584,'muted',48);
    button(waiting,'Отменить',424,122,176);
    const downloading=sample(exchange,'Получение БД / место и загрузка',672,112,624,220);
    text(downloading,'Файл БД',16,26,144,13,'muted');
    text(downloading,'~/Documents/Рабочая.taypeer',176,26,416,13);
    progress(downloading,'Получено 1,8 из 2,4 МиБ',16,80,592,0.75);
    button(downloading,'Отменить',416,170,192);
    const expiration=sample(exchange,'Приглашение истекло / управляющий',24,372,616,162);
    caption(expiration,'Приглашение истекло. Код и QR больше не действуют.',16,22,584,'warning',48);
    button(expiration,'Создать новое',360,108,240,'primary');
    const disconnected=sample(exchange,'Получение прервано',672,412,624,162);
    caption(disconnected,'Соединение прервано. Сохранённая часть загрузки\nостанется доступна для продолжения.',16,22,592,'muted',52);
    button(disconnected,'Продолжить',368,108,240,'primary');
    notice(exchange,'rejected invitation','Управляющее устройство отклонило подключение.',24,604,616,{action:'Ввести другой код',h:76});
    notice(exchange,'used invitation','Приглашение уже использовано.',672,644,624,{action:'Ввести другой код',h:76});
    notice(exchange,'sync wait','Ожидание Mac mini. Локальная работа доступна.',24,730,616,{action:'Повторить'});
    notice(exchange,'sync connecting','Соединение с Pixel 9 через relay…',672,770,624,{h:68});
    notice(exchange,'stored not applied','Изменения получены. Для применения войдите в БД.',24,838,616,{action:'Разблокировать',h:76});
    notice(exchange,'applied','Изменения применены · сегодня, 10:32',672,878,624,{tone:'success'});
    notice(exchange,'receive disk failure','Недостаточно места. Получение не завершено.',24,958,616,{tone:'danger',action:'Повторить',h:76});
    notice(exchange,'direct unavailable','Прямое соединение недоступно. Relay отключены.',672,998,624,{action:'Настройки…',h:76});
    notice(exchange,'camera denied','Камера недоступна. Можно ввести код приглашения.',24,1078,616,{action:'Ввести код',h:76});
    notice(exchange,'handoff waiting','Передача управления начата. Ожидается Pixel 9.',672,1118,624,{action:'Продолжить',h:76});

    const cases=sheet('States / conflicts','Конфликты и отложенные данные','Баннер остаётся в рабочей области. Разбор открывается явным действием.',1260);
    notice(cases,'workspace conflicts','Есть 3 конфликта. Все варианты сохранены.',24,112,1272,{action:'Разобрать…'});
    const deleted=sample(cases,'Удаление и параллельная правка',24,234,616,246);
    heading(deleted,'Wi-Fi · объект находится в корзине',16,20,584);
    caption(deleted,'Pixel 9 удалил запись. Mac mini изменил пароль.',16,66,584);
    secret(deleted,'••••••••••••',16,110,584);
    button(deleted,'Подтвердить удаление',16,188,280,'danger');
    button(deleted,'Восстановить…',336,188,264,'primary');
    const location=sample(cases,'Конкурирующее размещение',672,234,624,246);
    heading(location,'Сервисы · выберите группу назначения',16,20,592);
    radio(location,'Работа / Инфраструктура · MacBook Pro',16,64,592);
    radio(location,'Личное · Pixel 9',16,110,592);
    field(location,'Другая группа','Выбрать…',16,156,592,{select:true});
    button(location,'Применить',408,204,200,'disabled');
    const duplicateAttr=sample(cases,'Совпадающие ключи / защита атрибута',24,550,616,256);
    heading(duplicateAttr,'Два атрибута «Recovery code»',16,18,584);
    secret(duplicateAttr,'Защищён · ••••••••••••',16,66,584);
    secret(duplicateAttr,'Защищён · ••••••••••••',16,110,584);
    field(duplicateAttr,'Новое имя','Recovery code 2',16,164,584);
    button(duplicateAttr,'Применить',392,212,208,'primary');
    const revoked=sample(cases,'Зависимость от отозванного автора',672,550,624,256);
    caption(revoked,'Для переноса нужны ещё не принятые данные Pixel 9.\nОстальные допустимые правки уже применены.',16,22,592,'muted',58);
    button(revoked,'Просмотреть источник…',16,112,312);
    caption(revoked,'Источник остаётся до успешного переноса\nили отдельного удаления.',16,170,592,'muted',56);
    notice(cases,'late purged','Пришли правки окончательно очищенного объекта.\nОни сохранены отдельно и не возвращают объект автоматически.',24,888,616,{action:'Просмотреть…',h:100});
    notice(cases,'schema waiting','Правки сохранены и ожидают управляющее устройство.',672,888,624,{action:'Устройства',h:100});
    notice(cases,'control divergence','Обнаружено расхождение управления. Обмен остановлен.\nЛокальное чтение доступно.',24,1052,1272,{tone:'danger',action:'Восстановить…',h:84});

    const prefs=sheet('States / preferences','Настройки и ограничения роли','Параметры устройства применяются сразу; защищённые операции БД требуют отдельного подтверждения.',1260);
    menu(prefs,'Тема',24,112,392,[['Системная','','selected'],'Светлая','Тёмная']);
    menu(prefs,'Relay',448,112,392,[['Общие relay','','selected'],'Собственные relay','Отключены']);
    menu(prefs,'Очистка буфера',872,112,424,[['Через 30 секунд','','selected'],'Другой интервал…','Не очищать']);
    const timers=sample(prefs,'Произвольный интервал',24,330,616,196);
    field(timers,'Очистка буфера','45 секунд',16,24,584);
    field(timers,'Автоблокировка','10 минут',16,78,584);
    caption(timers,'Блокировка ОС или сон блокируют БД немедленно.',16,142,584);
    const relay=sample(prefs,'Собственные relay',672,330,624,196);
    field(relay,'Relay URL','https://relay.example.test',16,24,592);
    button(relay,'Добавить адрес',176,80,208);
    caption(relay,'Настройка относится к этому устройству.',16,142,592);
    const invalid=sample(prefs,'Ошибка настройки / значение не применено',24,602,616,182);
    field(invalid,'Очистка буфера','0',16,24,584,{error:true});
    caption(invalid,'Введите положительное число секунд\nили выберите «Не очищать».',176,76,408,'danger',52);
    const member=sample(prefs,'Обычное доверенное устройство',672,602,624,182);
    caption(member,'Управляющее устройство: MacBook Pro',16,20,592,'fg');
    button(member,'Поделиться…',16,70,208,'disabled');
    button(member,'Отозвать доступ…',240,70,240,'disabled');
    caption(member,'Приглашения, пароль, защита и общие лимиты\nменяются на управляющем устройстве.',16,120,592,'muted',48);
    const limits=sample(prefs,'Подтверждение общих лимитов',24,864,616,254);
    field(limits,'Одно вложение','20 МиБ',16,22,584);
    field(limits,'Все вложения','200 МиБ',16,70,584);
    caption(limits,'Изменения распространятся на доверенные устройства.\nМаксимум: 100 МиБ на файл и 1 ГиБ на БД.',16,122,584,'muted',54);
    button(limits,'Отмена',248,204,144); button(limits,'Применить',408,204,192,'primary');
    const format=sample(prefs,'Миграция / управляющий недоступен',672,864,624,254);
    notice(format,'migration role','Для обновления формата требуется\nуправляющее устройство MacBook Pro.',16,20,592,{h:84});
    button(format,'Обновить формат…',16,126,272,'disabled');
    button(format,'Устройства',352,126,256);
    caption(format,'Поддерживаемое локальное чтение доступно.',16,196,592);

    const entry=sheet('States / entry','Поля записи, вложения и оформление','Варианты в пределах общего черновика; исторические секреты раскрываются только по действию.',1540);
    const expiry=sample(entry,'Срок действия / включён',24,112,616,250);
    check(expiry,'Срок действия',16,16,584,true);
    field(expiry,'Дата','15.12.2026',16,66,584,{select:true});
    button(expiry,'Через месяц',176,124,176); button(expiry,'Через год',368,124,168);
    caption(expiry,'Выбрано: 15 декабря 2026',176,188,424);
    const attributeError=sample(entry,'Ошибка ключа атрибута',672,112,624,250);
    field(attributeError,'Ключ','Recovery code',16,24,592,{error:true});
    caption(attributeError,'Атрибут с таким именем уже существует.',176,76,416,'danger',44);
    field(attributeError,'Значение','••••••••••••',16,136,592,{secret:true});
    check(attributeError,'Защищённое значение',176,190,416,true);
    const blob=sample(entry,'Добавление сверх лимита',24,446,616,210);
    caption(blob,'archive.zip · 12 МиБ',16,22,584,'fg');
    notice(blob,'attachment too large','Размер превышает лимит 10 МиБ на вложение.',16,70,584,{tone:'danger',h:66});
    button(blob,'Выбрать другой…',304,156,296);
    const quota=sample(entry,'Превышение после объединения',672,446,624,210);
    caption(quota,'Вложения: 112 / 100 МиБ',16,22,592,'warning');
    caption(quota,'Принятые файлы сохранены. Новые добавления недоступны\nдо очистки или повышения общего лимита.',16,66,592,'muted',54);
    button(quota,'Добавить…',16,156,176,'disabled'); button(quota,'Настройки БД',368,156,240);
    const replaceFile=sample(entry,'Замена вложения',24,744,616,224);
    fileField(replaceFile,'Новый файл','recovery-codes-2026.txt',16,24,584);
    caption(replaceFile,'Содержимое будет заменено после сохранения записи.\nПредыдущая версия останется в истории.',16,86,584,'muted',56);
    button(replaceFile,'Заменить',392,170,208,'primary');
    const imageError=sample(entry,'Изображение / ошибка загрузки',672,744,624,224);
    field(imageError,'URL иконки','https://example.test/icon.png',16,24,592);
    notice(imageError,'image size','Изображение превышает 1 МиБ. Выберите другой файл.',16,82,592,{tone:'danger',h:72});
    button(imageError,'Из файла…',16,176,224); button(imageError,'Загрузить',368,176,240);
    notice(entry,'retained blobs','Вложение удалено из записи, но используется историей.\nЕго содержимое пока занимает место.',24,1058,616,{action:'История',h:88});
    notice(entry,'missing blob','Ожидается содержимое вложения. Сохранение наружу\nбудет доступно после его получения.',672,1058,624,{action:'Повторить обмен',h:88});
    const emptyAlphabet=sample(entry,'Генератор / пустой алфавит',24,1206,616,208);
    notice(emptyAlphabet,'empty alphabet','После исключений не осталось символов.\nИзмените наборы или исключения.',16,20,584,{tone:'danger',h:80});
    button(emptyAlphabet,'Сгенерировать',16,138,256,'disabled'); button(emptyAlphabet,'Использовать',320,138,280,'disabled');
    const copied=sample(entry,'Копирование / исходная запись сохранена',672,1206,624,208);
    notice(copied,'copied','Пароль скопирован. Очистка через 30 секунд.',16,20,592,{tone:'success',h:76});
    caption(copied,'Таймер удаляет только значение, скопированное Taypeer.\nПоследующее копирование другим приложением сохраняется.',16,124,592,'muted',64);

    const compatibility=sheet('States / compatibility','Совместимость, перенос и восстановление','Ограниченный доступ имеет явную причину. Исходные файлы сохраняются при ошибках.',1240);
    const readOnly=sample(compatibility,'Чтение без поддержки записи',24,112,616,212);
    notice(readOnly,'read only','Доступно только чтение.\nДля изменений обновите Taypeer.',16,20,584,{h:76});
    button(readOnly,'Редактировать',16,144,208,'disabled'); button(readOnly,'Копировать логин',352,144,248);
    const noAdmission=sample(compatibility,'Файл на новом устройстве / нет допуска',672,112,624,212);
    caption(noAdmission,'БД открыта для чтения. Этот Mac ещё не подключён\nк доверенному набору устройств.',16,22,592,'muted',56);
    button(noAdmission,'Подключить…',16,112,240); button(noAdmission,'Восстановить управление…',272,112,336);
    notice(compatibility,'unsupported read','Для открытия этой БД обновите Taypeer.\nИсходный файл не изменён.',24,410,616,{action:'Назад',h:88});
    notice(compatibility,'unsupported sync','Для обмена с Mac mini требуется совместимая версия Taypeer.',672,410,624,{action:'Устройства',h:88});
    notice(compatibility,'export conflicts','Экспорт недоступен: есть 3 неразрешённых конфликта.',24,566,616,{action:'Разобрать…',h:84});
    notice(compatibility,'unsupported kdbx','Этот KDBX требует неподдерживаемый аппаратный\nключ или компонент плагина.',672,566,624,{action:'Другой файл…',h:84});
    const importModes=sample(compatibility,'Импорт KDBX / только ключевой файл',24,734,616,202);
    field(importModes,'Способ открытия','Только ключевой файл',16,24,584,{select:true});
    fileField(importModes,'Ключевой файл','Личная.keyx',16,78,584);
    button(importModes,'Продолжить…',344,146,256,'primary');
    const operation=sample(compatibility,'Импорт / экспорт / снимок / миграция',672,734,624,202);
    progress(operation,'Сохранение результата…',16,22,592,0.65);
    button(operation,'Сохранение…',352,146,256,'disabled');
    notice(compatibility,'migration failure','Не удалось записать обновлённую БД.\nИсходная исправная копия сохранена.',24,1020,616,{tone:'danger',action:'Повторить',h:100});
    notice(compatibility,'transfer success','Файл ~/Documents/Рабочая.kdbx надёжно сохранён.',672,1020,624,{tone:'success',action:'Готово',h:100});
    const empty=art('macOS / empty').children.find(n=>n.name==='Table / entries');
    caption(empty,'Создайте группу для первой записи.',16,148,empty.width-32);
    const fresh=art('macOS / new-entry').findAll(n=>n.name==='Entry row / GitHub')[0];
    if(fresh)fill(fresh,'bg');
  }
  function buildReviewVariants() {
    const light = LIGHT_PALETTE;
    const lightCollection = figma.createVariableCollection('Taypeer · semantic / light');
    const lightVars = Object.fromEntries(Object.entries(light).map(([key,value])=>[key,figma.createVariable(key,'COLOR',lightCollection.id,{...rgb(value),a:1})]));
    function role(color) {
      return Object.entries(P).find(([,value]) => {const c=rgb(value);return ['r','g','b'].every(k=>Math.abs(color[k]-c[k])<0.001);})?.[0];
    }
    function recolor(n) {
      for(const prop of ['fills','strokes']) {
        const paints = n[prop];
        if(!Array.isArray(paints)||!paints.length)continue;
        const roles=paints.map(p=>p.type==='SOLID'?role(p.color):null);
        n[prop]=paints.map((p,i)=>roles[i]?{...p,color:rgb(light[roles[i]])}:p);
        roles.forEach((r,i)=>{if(r)figma.bindVariable(n.id,`${prop}/${i}/color`,lightVars[r].id);});
      }
      if(n.children)n.children.forEach(recolor);
    }
    for(const [source,name] of [['macOS / main','main'],['macOS / edit','edit'],['macOS / settings','settings'],['macOS / unlock','unlock'],['Dialog / create','create']]) {
      recolor(duplicate(source,'QA / light-'+name,qaPage));
    }
    function sizePreview(name,w,h,font) {
      const p=board(qaPage,'QA / '+name,w,h);
      const side=w===1100?192:224, list=w===1100?426:480, dx=side+list+2, dw=w-dx;
      box(p,'TitleBar',0,0,w,44,'chrome');text(p,'Рабочая.taypeer',w/2-96,12,232,14,'fg',500);ib(p,'settings',w-40,6);
      ['#EA6B66','#D6B86C','#80B981'].forEach((c,i)=>box(p,'Window control',16+i*19,16,11,11,c,6));
      box(p,'Sidebar',0,45,side,h-74,'panel');rule(p,side,45,1,h-74);
      ib(p,'folder-plus',side-40,51);rule(p,0,89,side);
      [['Работа',0],['Сервисы',1],['Личное',0]].forEach(([label,depth],i)=>{
        const y=100+i*40;
        if(i===0)box(p,'Selected group',8,y,side-16,36,'selected',4);
        icon(p,'folder',16+depth*16,y+10);text(p,label,40+depth*16,y+7,side-64-depth*16,font,'fg',400,28);
      });
      [['trash-2','Корзина'],['monitor','Устройства'],['history','Копии']].forEach(([key,label],i)=>{
        icon(p,key,16,h-176+i*40);text(p,label,44,h-180+i*40,side-56,font,'muted',400,30);
      });
      const tableX=side+1;
      const search=inputFrame(p,'Search',tableX+12,51,list-96);icon(search,'search',8,8);text(search,'Поиск',36,3,list-148,font,'muted',400,26);
      ib(p,'file-plus-2',tableX+list-76,51);ib(p,'ellipsis',tableX+list-40,51);rule(p,tableX,89,list);
      const columns=w===1100?[['Название',list*0.53],['База / группа',list*0.47]]:[['Название',170],['Логин',124],['База / группа',list-294]];
      let xx=tableX;
      columns.forEach(([label,width],i)=>{text(p,label,xx+12,99,width-36,font-1,'muted',500,28);icon(p,i===0?'arrow-up':'chevrons-up-down',xx+width-24,102);if(i)rule(p,xx,90,1,37);xx+=width;});
      rule(p,tableX,127,list);
      [['Cloudflare','admin'],['Figma','valentin'],['GitHub','valentin'],['Linear','valentin']].forEach(([title,user],i)=>{
        const y=128+i*44;box(p,'Entry / '+title,tableX,y,list,44,i===2?'selected':i%2?'panel':'bg');
        let x=tableX;const values=w===1100?[title,'Рабочая / Работа']:[title,user,'Работа'];
        values.forEach((value,j)=>{text(p,value,x+12,y+10,columns[j][1]-24,j?font-2:font,j?'muted':'fg',400,28);x+=columns[j][1];});rule(p,tableX,y+43,list);
      });
      rule(p,dx-1,45,1,h-74);heading(p,'GitHub',dx+24,55,dw-144);ib(p,'x',w-96,51);ib(p,'check',w-56,51);rule(p,dx,89,dw);
      const tabLabels=['Обзор','Дополнительно','Вид','Свойства','История'];
      const widths=font>=18?[76,176,58,134,122]:[68,132,54,108,92];let tx=dx;
      tabLabels.forEach((label,i)=>{text(p,label,tx+6,99,widths[i]-8,font-1,i===0?'fg':'muted',400,28);tx+=widths[i];});
      rule(p,dx,127,dw);box(p,'Selected tab',dx,126,widths[0],2,'muted');
      const values=[['Название','GitHub'],['Логин','valentin'],['Пароль','••••••••••'],['URL','https://github.com'],['Теги','работа'],['Заметки','Рабочий аккаунт'],['Срок действия','Бессрочно']];
      values.forEach(([label,value],i)=>{
        const y=128+i*48;box(p,'Description row / '+label,dx,y,dw,48,i%2?'panel':'bg');
        text(p,label,dx+24,y+10,144,font-1,'muted',400,28);
        const control=inputFrame(p,label,dx+184,y+5,dw-208,38,'row');text(control,value,0,7,control.width-42,font,'fg',400,28);
        if(i===2)ib(control,'dice-5',control.width-34,3);if(i===6)icon(control,'chevron-down',control.width-24,12);
        rule(p,dx,y+47,dw);
      });
      box(p,'StatusBar',0,h-28,w,28,'panel');rule(p,0,h-28,w);text(p,'2,4 МиБ',16,h-22,160,11,'muted');text(p,'Синхронизировано',w-250,h-22,210,11,'muted').textAlignHorizontal='RIGHT';icon(p,'refresh-cw',w-28,h-22);
      return p;
    }
    sizePreview('minimum-1100-720',1100,720,14);
    sizePreview('font-16',1320,820,16);
    sizePreview('font-18',1320,820,18);
  }
  function buildFlowMap() {
    const p=board(flowPage,'Flow / macOS-v1',1320,1780);
    text(p,'macOS · карта сценариев v1',24,24,1272,24,'fg',500,36);
    caption(p,'Точки входа, решения и результаты. Карта относится к макетам; готовность кода определяется roadmap.',24,78,1272);
    const flows=[
      ['01 · Начало работы','Приветствие → создать / открыть\n→ разблокировать → пустая БД\n→ первая группа → новая запись','welcome · create · unlock\nempty · group · new-entry'],
      ['02 · Повседневная работа','Группа → запись → вкладки\n→ правки → подтверждение\n→ новая сохранённая версия','main · edit · Вкладки записи\nMenus / objects'],
      ['03 · Навигация и поиск','Селектор БД → область поиска\n→ результат в другой БД\n→ к исходным результатам','search · Menus / navigation\nsettings-locked'],
      ['04 · История и корзина','История → сравнение → восстановить\nУдаление → корзина → восстановить\nНет группы → выбор назначения','history-compare · trash\nrestore-destination'],
      ['05 · Вложения и оформление','Правки → атрибут / вложение\n→ иконка / цвет → сохранить\nЛимит → очистка / настройки','attribute · icon-picker · color-picker\nStates / entry'],
      ['06 · Генерация','Поле пароля → генератор\n→ пароль / фраза → настройки\n→ использовать в черновике','generator · passphrase\ngenerator-exclusions'],
      ['07 · Получение БД','Поделиться → код / QR\n→ получатель → разрешить\n→ файл → загрузка → вход','invite · receive · scan-qr\nStates / exchange · unlock'],
      ['08 · Управление доступом','Устройства → передать / отозвать\nНовый пароль → новая защита\nПотеря управляющего → новый набор','transfer-control · accept-control\nrevoke · recover-control'],
      ['09 · Получение и применение','Ожидание → соединение → получение\n→ надёжное сохранение → вход\n→ применение / отложенные правки','devices · States / exchange\npending · conflict'],
      ['10 · Разбор изменений','Уведомление → конфликт → выбор\nОтложенный источник → просмотр\n→ извлечь / явно удалить','conflict · pending\nStates / conflicts'],
      ['11 · Перенос и снимки','KDBX → параметры → отчёт → файл\nСнимок → просмотр → подтверждение\n→ новые данные в текущей БД','transfer · import-review · transfer-report\nbackups · restore-backup'],
      ['12 · Совместимость и защита','Открытие → чтение / обновить\nУправляющий → копия → миграция\nРасхождение → восстановить управление','States / compatibility · migrate\nsettings-database · recover-control']
    ];
    flows.forEach(([title,steps,refs],i)=>{
      const x=24+i%3*432,y=144+Math.floor(i/3)*326;
      const n=box(p,'Flow / '+title,x,y,408,290,'panel',4,true);
      heading(n,title,16,18,376);
      caption(n,steps,16,66,376,'fg',112);
      rule(n,16,192,376);
      caption(n,refs,16,214,376,'muted',56);
    });
    heading(p,'Общие правила перехода',24,1482,1272);
    caption(p,'Изменённая форма → Сохранить / Не сохранять / Остаться. Ошибка сохранения оставляет форму открытой.\nБлокировка выполняется сразу; после входа предлагается сохранённый локальный черновик.\nНативные окна macOS: выбор файла, каталога, камера и Touch ID. Отмена возвращает в вызвавший экран.',24,1532,1272,'muted',96);
    caption(p,'Полная матрица требований и состояний: wireframes/COVERAGE.md',24,1676,1272,'muted',36);
    replace('Taypeer / обзор',()=>{
      const n=board('00 · Начать здесь','Taypeer / обзор',1320,950);
      text(n,'Taypeer',32,32,1256,32,'fg',500,46);
      heading(n,'macOS · полный набор сценариев первого релиза',32,100,1256);
      const rows=[['01 · macOS','Рабочие экраны и новые задачи: корзина, история, получение БД, отложенные правки'],['02 · Android','Исходные мобильные макеты'],['03 · Диалоги','Формы и подтверждения: данные, устройства, перенос, восстановление'],['04–05 · Компоненты и вкладки','Базовые компоненты, семантические цвета, общая область правок'],['06 · Состояния и меню','Ошибки, ожидание, права, совместимость, раскрытые меню'],['07 · Тема и размеры','Светлая тема, минимальное окно, текст 16 и 18'],['08 · Карта сценариев','Путь от точки входа до подтверждённого результата']];
      rows.forEach(([a,b],i)=>{const y=178+i*92;rule(n,32,y,1256);heading(n,a,32,y+16,1256);caption(n,b,32,y+48,1256);});
      caption(n,'Статические редактируемые макеты · русский язык · искусственные данные\nПокрытие требований и поведение: COVERAGE.md и README.md',32,846,1256,'muted',64);
      return n;
    });
  }
  function arrangePages() {
    // Use each row's largest artboard; dialogs and review windows vary in size.
    // This replaces the old fixed-height grid, which would overlap taller forms.
    for(const p of Object.values(pages)) {
      if(p.name==='02 · Android') {counts[p.name]=p.children.length;continue;}
      const nodes=[...p.children];
      const columns=p.name==='03 · Диалоги'?3:2;
      const colWidth=Math.max(...nodes.map(n=>n.width),0)+80;
      let y=0;
      for(let i=0;i<nodes.length;i+=columns) {
        const row=nodes.slice(i,i+columns);
        row.forEach((n,j)=>{n.x=j*colWidth;n.y=y;});
        y+=Math.max(...row.map(n=>n.height))+80;
      }
      counts[p.name]=nodes.length;
    }
  }
}
