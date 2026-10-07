//! Allocation-free system localization shared by Genesis and the desktop.

use core::sync::atomic::{AtomicU8, Ordering};

static ACTIVE: AtomicU8 = AtomicU8::new(Locale::English as u8);

pub fn set_active(locale: Locale) {
    ACTIVE.store(locale.persisted(), Ordering::Release);
}

pub fn active() -> Locale {
    Locale::from_persisted(ACTIVE.load(Ordering::Acquire))
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Locale {
    #[default]
    English = 0,
    Russian = 1,
    Hebrew = 2,
    German = 3,
    Esperanto = 4,
}

impl Locale {
    pub const ALL: [Self; 5] = [
        Self::English,
        Self::Russian,
        Self::Hebrew,
        Self::German,
        Self::Esperanto,
    ];

    pub const fn from_persisted(value: u8) -> Self {
        match value {
            1 => Self::Russian,
            2 => Self::Hebrew,
            3 => Self::German,
            4 => Self::Esperanto,
            _ => Self::English,
        }
    }

    pub const fn persisted(self) -> u8 {
        self as u8
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::Russian => "Русский",
            Self::Hebrew => "עברית",
            Self::German => "Deutsch",
            Self::Esperanto => "Esperanto",
        }
    }

    pub const fn is_rtl(self) -> bool {
        matches!(self, Self::Hebrew)
    }

    pub const fn text(self, key: Text) -> &'static str {
        let row = key as usize;
        match self {
            Self::English => ENGLISH[row],
            Self::Russian => RUSSIAN[row],
            Self::Hebrew => HEBREW[row],
            Self::German => GERMAN[row],
            Self::Esperanto => ESPERANTO[row],
        }
    }

    pub const fn category(self, index: usize) -> &'static str {
        match self {
            Self::English => CATEGORIES_EN[index],
            Self::Russian => CATEGORIES_RU[index],
            Self::Hebrew => CATEGORIES_HE[index],
            Self::German => CATEGORIES_DE[index],
            Self::Esperanto => CATEGORIES_EO[index],
        }
    }

    pub const fn app(self, index: usize) -> &'static str {
        match self {
            Self::English => APPS_EN[index],
            Self::Russian => APPS_RU[index],
            Self::Hebrew => APPS_HE[index],
            Self::German => APPS_DE[index],
            Self::Esperanto => APPS_EO[index],
        }
    }
}

#[repr(usize)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(not(feature = "genesis-installer"), allow(dead_code))]
pub enum Text {
    Settings,
    OpenShell,
    DesktopWillClose,
    Open,
    Cancel,
    Selected,
    Available,
    SystemInstallerLanguage,
    RtlInterface,
    LanguagePrompt,
    InstallationMode,
    BasicEncrypted,
    ArchitectPreview,
    CfcName,
    PrimaryName,
    OperatorPassword,
    ConfirmPassword,
    InstallationPlan,
    DestructiveWarning,
    TypeErase,
    Cancelled,
    Complete,
}

const ENGLISH: [&str; 22] = [
    "Settings",
    "Open command shell?",
    "The desktop will close safely.",
    "OPEN",
    "CANCEL",
    "Selected",
    "Available",
    "System and installer language",
    "Right-to-left interface",
    "Language [1-5]: ",
    "Installation mode [1/2]: ",
    "Basic encrypted",
    "Architect preview",
    "CFC name: ",
    "Primary Dimension name: ",
    "Operator password: ",
    "Confirm password: ",
    "Installation plan",
    "WARNING: the target disk will be replaced.",
    "Type ERASE to continue: ",
    "Installation cancelled; no disk writes were made.",
    "Installation complete",
];
const RUSSIAN: [&str; 22] = [
    "Настройки",
    "Открыть командную оболочку?",
    "Рабочий стол будет безопасно закрыт.",
    "ОТКРЫТЬ",
    "ОТМЕНА",
    "Выбрано",
    "Доступно",
    "Язык системы и установщика",
    "Интерфейс справа налево",
    "Язык [1-5]: ",
    "Режим установки [1/2]: ",
    "Базовый с шифрованием",
    "Режим архитектора",
    "Имя CFC: ",
    "Имя основного Измерения: ",
    "Пароль оператора: ",
    "Подтвердите пароль: ",
    "План установки",
    "ВНИМАНИЕ: целевой диск будет заменён.",
    "Введите ERASE для продолжения: ",
    "Установка отменена; диск не изменён.",
    "Установка завершена",
];
const HEBREW: [&str; 22] = [
    "הגדרות",
    "לפתוח מעטפת פקודות?",
    "שולחן העבודה ייסגר בבטחה.",
    "פתיחה",
    "ביטול",
    "נבחר",
    "זמין",
    "שפת המערכת וההתקנה",
    "ממשק מימין לשמאל",
    "שפה [1-5]: ",
    "מצב התקנה [1/2]: ",
    "בסיסי מוצפן",
    "מצב אדריכל",
    "שם CFC: ",
    "שם הממד הראשי: ",
    "סיסמת מפעיל: ",
    "אישור סיסמה: ",
    "תוכנית התקנה",
    "אזהרה: דיסק היעד יוחלף.",
    "יש להקליד ERASE כדי להמשיך: ",
    "ההתקנה בוטלה; הדיסק לא שונה.",
    "ההתקנה הושלמה",
];
const GERMAN: [&str; 22] = [
    "Einstellungen",
    "Befehls-Shell öffnen?",
    "Der Desktop wird sicher geschlossen.",
    "ÖFFNEN",
    "ABBRECHEN",
    "Ausgewählt",
    "Verfügbar",
    "System- und Installationssprache",
    "Rechts-nach-links-Oberfläche",
    "Sprache [1-5]: ",
    "Installationsmodus [1/2]: ",
    "Basis verschlüsselt",
    "Architektenmodus",
    "CFC-Name: ",
    "Name der primären Dimension: ",
    "Operator-Passwort: ",
    "Passwort bestätigen: ",
    "Installationsplan",
    "WARNUNG: Der Zieldatenträger wird ersetzt.",
    "ERASE zum Fortfahren eingeben: ",
    "Installation abgebrochen; Datenträger unverändert.",
    "Installation abgeschlossen",
];
const ESPERANTO: [&str; 22] = [
    "Agordoj",
    "Ĉu malfermi komandŝelon?",
    "La labortablo sekure fermiĝos.",
    "MALFERMI",
    "NULIGI",
    "Elektita",
    "Disponebla",
    "Lingvo de sistemo kaj instalilo",
    "Interfaco dekstre-maldekstren",
    "Lingvo [1-5]: ",
    "Instala reĝimo [1/2]: ",
    "Baza ĉifrita",
    "Arkitekta reĝimo",
    "Nomo de CFC: ",
    "Nomo de ĉefa Dimensio: ",
    "Pasvorto de operatoro: ",
    "Konfirmu pasvorton: ",
    "Instala plano",
    "AVERTO: la cela disko estos anstataŭigita.",
    "Tajpu ERASE por daŭrigi: ",
    "Instalado nuligita; la disko ne ŝanĝiĝis.",
    "Instalado finiĝis",
];

const CATEGORIES_EN: [&str; 18] = [
    "System",
    "Appearance",
    "Network & Wi-Fi",
    "Bluetooth",
    "Display",
    "Audio",
    "Performance",
    "Mouse & keyboard",
    "Windows",
    "Taskbar",
    "Menu",
    "Profiles",
    "Accessibility",
    "Privacy",
    "About",
    "Command shell",
    "Language & region",
    "Date & time",
];
const CATEGORIES_RU: [&str; 18] = [
    "Система",
    "Внешний вид",
    "Сеть и Wi-Fi",
    "Bluetooth",
    "Экран",
    "Звук",
    "Производительность",
    "Мышь и клавиатура",
    "Окна",
    "Панель задач",
    "Меню",
    "Профили",
    "Доступность",
    "Конфиденциальность",
    "О системе",
    "Командная оболочка",
    "Язык и регион",
    "Дата и время",
];
const CATEGORIES_HE: [&str; 18] = [
    "מערכת",
    "מראה",
    "רשת ו-Wi-Fi",
    "Bluetooth",
    "תצוגה",
    "שמע",
    "ביצועים",
    "עכבר ומקלדת",
    "חלונות",
    "שורת משימות",
    "תפריט",
    "פרופילים",
    "נגישות",
    "פרטיות",
    "אודות",
    "מעטפת פקודות",
    "שפה ואזור",
    "תאריך ושעה",
];
const CATEGORIES_DE: [&str; 18] = [
    "System",
    "Darstellung",
    "Netzwerk & Wi-Fi",
    "Bluetooth",
    "Anzeige",
    "Audio",
    "Leistung",
    "Maus & Tastatur",
    "Fenster",
    "Taskleiste",
    "Menü",
    "Profile",
    "Barrierefreiheit",
    "Datenschutz",
    "Info",
    "Befehls-Shell",
    "Sprache & Region",
    "Datum & Uhrzeit",
];
const CATEGORIES_EO: [&str; 18] = [
    "Sistemo",
    "Aspekto",
    "Reto kaj Wi-Fi",
    "Bluetooth",
    "Ekrano",
    "Sono",
    "Efikeco",
    "Muso kaj klavaro",
    "Fenestroj",
    "Taskobreto",
    "Menuo",
    "Profiloj",
    "Alirebleco",
    "Privateco",
    "Pri",
    "Komandŝelo",
    "Lingvo kaj regiono",
    "Dato kaj horo",
];

const APPS_EN: [&str; 9] = [
    "Browser", "Terminal", "Forms", "Ayo", "Settings", "System", "Games", "Notes", "Apps",
];
const APPS_RU: [&str; 9] = [
    "Браузер",
    "Терминал",
    "Формы",
    "Ayo",
    "Настройки",
    "Система",
    "Игры",
    "Заметки",
    "Приложения",
];
const APPS_HE: [&str; 9] = [
    "דפדפן",
    "מסוף",
    "טפסים",
    "Ayo",
    "הגדרות",
    "מערכת",
    "משחקים",
    "הערות",
    "יישומים",
];
const APPS_DE: [&str; 9] = [
    "Browser",
    "Terminal",
    "Formen",
    "Ayo",
    "Einstellungen",
    "System",
    "Spiele",
    "Notizen",
    "Apps",
];
const APPS_EO: [&str; 9] = [
    "Retumilo",
    "Terminalo",
    "Formoj",
    "Ayo",
    "Agordoj",
    "Sistemo",
    "Ludoj",
    "Notoj",
    "Apoj",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_locales_and_rtl_policy_are_stable() {
        for (id, locale) in Locale::ALL.iter().copied().enumerate() {
            assert_eq!(Locale::from_persisted(id as u8), locale);
            assert_eq!(locale.persisted(), id as u8);
            assert!(!locale.name().is_empty());
        }
        assert!(Locale::Hebrew.is_rtl());
        assert!(!Locale::Russian.is_rtl());
    }
}
