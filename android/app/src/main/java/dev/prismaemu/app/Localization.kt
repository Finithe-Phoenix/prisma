package dev.prismaemu.app

import android.content.Context
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.unit.LayoutDirection

enum class UiText {
    Home,
    Library,
    Activity,
    Settings,
    HeroTitle,
    HeroSubtitle,
    PreviewMode,
    ImportApp,
    TryTranslation,
    RecentWorkspaces,
    ViewAll,
    Tools,
    Language,
    LanguageDescription,
    SearchLanguages,
    Current,
    Appearance,
    About,
    LibraryDescription,
    ActivityDescription,
    NoActivity,
    SettingsDescription,
    NewWorkspace,
    Open,
    ComingSoon,
    EngineReady,
    Apps,
}

data class AppLanguage(
    val tag: String,
    val autonym: String,
    val englishName: String,
    val isRtl: Boolean = false,
)

class LanguagePack internal constructor(private val values: List<String>) {
    init {
        require(values.size == UiText.entries.size) {
            "Language pack has ${values.size} values; expected ${UiText.entries.size}"
        }
    }

    operator fun get(key: UiText): String = values[key.ordinal]
}

object PrismaLanguages {
    val supported = listOf(
        AppLanguage("en", "English", "English"),
        AppLanguage("es", "Español", "Spanish"),
        AppLanguage("fr", "Français", "French"),
        AppLanguage("de", "Deutsch", "German"),
        AppLanguage("it", "Italiano", "Italian"),
        AppLanguage("pt-BR", "Português (Brasil)", "Portuguese (Brazil)"),
        AppLanguage("nl", "Nederlands", "Dutch"),
        AppLanguage("pl", "Polski", "Polish"),
        AppLanguage("cs", "Čeština", "Czech"),
        AppLanguage("sk", "Slovenčina", "Slovak"),
        AppLanguage("hu", "Magyar", "Hungarian"),
        AppLanguage("ro", "Română", "Romanian"),
        AppLanguage("bg", "Български", "Bulgarian"),
        AppLanguage("el", "Ελληνικά", "Greek"),
        AppLanguage("ru", "Русский", "Russian"),
        AppLanguage("uk", "Українська", "Ukrainian"),
        AppLanguage("tr", "Türkçe", "Turkish"),
        AppLanguage("ar", "العربية", "Arabic", isRtl = true),
        AppLanguage("he", "עברית", "Hebrew", isRtl = true),
        AppLanguage("fa", "فارسی", "Persian", isRtl = true),
        AppLanguage("ur", "اردو", "Urdu", isRtl = true),
        AppLanguage("hi", "हिन्दी", "Hindi"),
        AppLanguage("bn", "বাংলা", "Bengali"),
        AppLanguage("ta", "தமிழ்", "Tamil"),
        AppLanguage("te", "తెలుగు", "Telugu"),
        AppLanguage("mr", "मराठी", "Marathi"),
        AppLanguage("gu", "ગુજરાતી", "Gujarati"),
        AppLanguage("pa", "ਪੰਜਾਬੀ", "Punjabi"),
        AppLanguage("id", "Bahasa Indonesia", "Indonesian"),
        AppLanguage("ms", "Bahasa Melayu", "Malay"),
        AppLanguage("vi", "Tiếng Việt", "Vietnamese"),
        AppLanguage("th", "ไทย", "Thai"),
        AppLanguage("fil", "Filipino", "Filipino"),
        AppLanguage("zh-CN", "简体中文", "Chinese (Simplified)"),
        AppLanguage("zh-TW", "繁體中文", "Chinese (Traditional)"),
        AppLanguage("ja", "日本語", "Japanese"),
        AppLanguage("ko", "한국어", "Korean"),
        AppLanguage("sv", "Svenska", "Swedish"),
        AppLanguage("no", "Norsk", "Norwegian"),
        AppLanguage("da", "Dansk", "Danish"),
        AppLanguage("fi", "Suomi", "Finnish"),
        AppLanguage("et", "Eesti", "Estonian"),
        AppLanguage("lv", "Latviešu", "Latvian"),
        AppLanguage("lt", "Lietuvių", "Lithuanian"),
        AppLanguage("sl", "Slovenščina", "Slovenian"),
        AppLanguage("hr", "Hrvatski", "Croatian"),
        AppLanguage("sr-Latn", "Srpski", "Serbian"),
        AppLanguage("bs", "Bosanski", "Bosnian"),
        AppLanguage("mk", "Македонски", "Macedonian"),
        AppLanguage("sq", "Shqip", "Albanian"),
        AppLanguage("sw", "Kiswahili", "Swahili"),
        AppLanguage("af", "Afrikaans", "Afrikaans"),
    )

    private fun decodePack(value: String) = LanguagePack(value.split('|'))

    private val packs = mapOf(
        "en" to decodePack("Home|Library|Activity|Settings|Windows, without limits.|Run x86-64 software on ARM64 Android.|Preview mode|Import app|Try translation|Recent workspaces|View all|Tools|Language|Choose the app language|Search languages|Current|Appearance|About Prisma|Your Windows apps and workspaces|Translation and runtime events|No runtime activity yet|Make Prisma yours|New workspace|Open|Coming soon|Engine ready|apps"),
        "es" to decodePack("Inicio|Biblioteca|Actividad|Ajustes|Windows, sin límites.|Ejecuta software x86-64 en Android ARM64.|Modo de vista previa|Importar aplicación|Probar traducción|Espacios recientes|Ver todo|Herramientas|Idioma|Elige el idioma de la aplicación|Buscar idiomas|Actual|Apariencia|Acerca de Prisma|Tus aplicaciones y espacios de Windows|Eventos de traducción y ejecución|Aún no hay actividad|Haz Prisma tuyo|Nuevo espacio|Abrir|Próximamente|Motor listo|aplicaciones"),
        "fr" to decodePack("Accueil|Bibliothèque|Activité|Paramètres|Windows, sans limites.|Exécutez des logiciels x86-64 sur Android ARM64.|Mode aperçu|Importer une application|Tester la traduction|Espaces récents|Tout voir|Outils|Langue|Choisissez la langue de l’application|Rechercher des langues|Actuelle|Apparence|À propos de Prisma|Vos applications et espaces Windows|Événements de traduction et d’exécution|Aucune activité pour le moment|Personnalisez Prisma|Nouvel espace|Ouvrir|Bientôt disponible|Moteur prêt|applications"),
        "de" to decodePack("Start|Bibliothek|Aktivität|Einstellungen|Windows, ohne Grenzen.|x86-64-Software auf ARM64-Android ausführen.|Vorschaumodus|App importieren|Übersetzung testen|Letzte Arbeitsbereiche|Alle anzeigen|Werkzeuge|Sprache|App-Sprache auswählen|Sprachen suchen|Aktuell|Darstellung|Über Prisma|Deine Windows-Apps und Arbeitsbereiche|Übersetzungs- und Laufzeitereignisse|Noch keine Aktivität|Prisma anpassen|Neuer Arbeitsbereich|Öffnen|Demnächst|Engine bereit|Apps"),
        "it" to decodePack("Home|Libreria|Attività|Impostazioni|Windows, senza limiti.|Esegui software x86-64 su Android ARM64.|Modalità anteprima|Importa app|Prova traduzione|Spazi recenti|Vedi tutto|Strumenti|Lingua|Scegli la lingua dell’app|Cerca lingue|Attuale|Aspetto|Informazioni su Prisma|Le tue app e aree Windows|Eventi di traduzione e runtime|Nessuna attività per ora|Personalizza Prisma|Nuovo spazio|Apri|Prossimamente|Motore pronto|app"),
        "pt-BR" to decodePack("Início|Biblioteca|Atividade|Configurações|Windows, sem limites.|Execute software x86-64 no Android ARM64.|Modo de prévia|Importar aplicativo|Testar tradução|Espaços recentes|Ver tudo|Ferramentas|Idioma|Escolha o idioma do aplicativo|Buscar idiomas|Atual|Aparência|Sobre o Prisma|Seus aplicativos e espaços Windows|Eventos de tradução e execução|Nenhuma atividade ainda|Deixe o Prisma do seu jeito|Novo espaço|Abrir|Em breve|Motor pronto|aplicativos"),
        "nl" to decodePack("Start|Bibliotheek|Activiteit|Instellingen|Windows, zonder grenzen.|Voer x86-64-software uit op ARM64 Android.|Voorbeeldmodus|App importeren|Vertaling proberen|Recente werkruimtes|Alles bekijken|Hulpmiddelen|Taal|Kies de app-taal|Talen zoeken|Huidig|Weergave|Over Prisma|Je Windows-apps en werkruimtes|Vertaal- en runtimegebeurtenissen|Nog geen activiteit|Maak Prisma van jou|Nieuwe werkruimte|Openen|Binnenkort|Engine gereed|apps"),
        "pl" to decodePack("Start|Biblioteka|Aktywność|Ustawienia|Windows bez ograniczeń.|Uruchamiaj oprogramowanie x86-64 na Androidzie ARM64.|Tryb podglądu|Importuj aplikację|Wypróbuj translację|Ostatnie obszary|Zobacz wszystko|Narzędzia|Język|Wybierz język aplikacji|Szukaj języków|Bieżący|Wygląd|O Prisma|Twoje aplikacje i obszary Windows|Zdarzenia translacji i wykonania|Brak aktywności|Dostosuj Prismę|Nowy obszar|Otwórz|Wkrótce|Silnik gotowy|aplikacje"),
        "cs" to decodePack("Domů|Knihovna|Aktivita|Nastavení|Windows bez omezení.|Spouštějte software x86-64 na Androidu ARM64.|Režim náhledu|Importovat aplikaci|Vyzkoušet překlad|Nedávné prostory|Zobrazit vše|Nástroje|Jazyk|Zvolte jazyk aplikace|Hledat jazyky|Aktuální|Vzhled|O aplikaci Prisma|Vaše aplikace a prostory Windows|Události překladu a běhu|Zatím žádná aktivita|Přizpůsobte si Prismu|Nový prostor|Otevřít|Již brzy|Engine je připraven|aplikace"),
        "sk" to decodePack("Domov|Knižnica|Aktivita|Nastavenia|Windows bez obmedzení.|Spúšťajte softvér x86-64 na Androide ARM64.|Režim ukážky|Importovať aplikáciu|Vyskúšať preklad|Nedávne priestory|Zobraziť všetko|Nástroje|Jazyk|Vyberte jazyk aplikácie|Hľadať jazyky|Aktuálny|Vzhľad|O aplikácii Prisma|Vaše aplikácie a priestory Windows|Udalosti prekladu a behu|Zatiaľ žiadna aktivita|Prispôsobte si Prismu|Nový priestor|Otvoriť|Už čoskoro|Engine je pripravený|aplikácie"),
        "hu" to decodePack("Kezdőlap|Könyvtár|Tevékenység|Beállítások|Windows, korlátok nélkül.|Futtass x86-64 szoftvert ARM64 Androidon.|Előnézeti mód|Alkalmazás importálása|Fordítás kipróbálása|Legutóbbi munkaterek|Összes megtekintése|Eszközök|Nyelv|Válaszd ki az alkalmazás nyelvét|Nyelvek keresése|Jelenlegi|Megjelenés|A Prismáról|Windows-alkalmazásaid és munkatereid|Fordítási és futási események|Még nincs tevékenység|Tedd sajátoddá a Prismát|Új munkatér|Megnyitás|Hamarosan|Motor kész|alkalmazás"),
        "ro" to decodePack("Acasă|Bibliotecă|Activitate|Setări|Windows, fără limite.|Rulează software x86-64 pe Android ARM64.|Mod previzualizare|Importă aplicație|Testează traducerea|Spații recente|Vezi tot|Instrumente|Limbă|Alege limba aplicației|Caută limbi|Curentă|Aspect|Despre Prisma|Aplicațiile și spațiile tale Windows|Evenimente de traducere și execuție|Încă nu există activitate|Personalizează Prisma|Spațiu nou|Deschide|În curând|Motor pregătit|aplicații"),
        "bg" to decodePack("Начало|Библиотека|Активност|Настройки|Windows без ограничения.|Стартирайте x86-64 софтуер на ARM64 Android.|Режим за преглед|Импортиране на приложение|Проба на превода|Скорошни пространства|Виж всички|Инструменти|Език|Изберете език на приложението|Търсене на езици|Текущ|Облик|За Prisma|Вашите Windows приложения и пространства|Събития за превод и изпълнение|Все още няма активност|Направете Prisma ваша|Ново пространство|Отвори|Очаквайте скоро|Енджинът е готов|приложения"),
        "el" to decodePack("Αρχική|Βιβλιοθήκη|Δραστηριότητα|Ρυθμίσεις|Windows, χωρίς όρια.|Εκτελέστε λογισμικό x86-64 σε ARM64 Android.|Λειτουργία προεπισκόπησης|Εισαγωγή εφαρμογής|Δοκιμή μετάφρασης|Πρόσφατοι χώροι|Προβολή όλων|Εργαλεία|Γλώσσα|Επιλέξτε τη γλώσσα της εφαρμογής|Αναζήτηση γλωσσών|Τρέχουσα|Εμφάνιση|Σχετικά με το Prisma|Οι εφαρμογές και οι χώροι Windows σας|Συμβάντα μετάφρασης και εκτέλεσης|Δεν υπάρχει ακόμη δραστηριότητα|Κάντε το Prisma δικό σας|Νέος χώρος|Άνοιγμα|Σύντομα|Η μηχανή είναι έτοιμη|εφαρμογές"),
        "ru" to decodePack("Главная|Библиотека|Активность|Настройки|Windows без границ.|Запускайте ПО x86-64 на ARM64 Android.|Режим предпросмотра|Импорт приложения|Проверить трансляцию|Недавние пространства|Показать все|Инструменты|Язык|Выберите язык приложения|Поиск языков|Текущий|Оформление|О Prisma|Ваши приложения и пространства Windows|События трансляции и выполнения|Активности пока нет|Настройте Prisma под себя|Новое пространство|Открыть|Скоро|Движок готов|приложения"),
        "uk" to decodePack("Головна|Бібліотека|Активність|Налаштування|Windows без обмежень.|Запускайте програми x86-64 на ARM64 Android.|Режим перегляду|Імпортувати програму|Спробувати трансляцію|Недавні простори|Переглянути всі|Інструменти|Мова|Виберіть мову програми|Пошук мов|Поточна|Вигляд|Про Prisma|Ваші програми та простори Windows|Події трансляції та виконання|Активності ще немає|Налаштуйте Prisma для себе|Новий простір|Відкрити|Незабаром|Рушій готовий|програми"),
        "tr" to decodePack("Ana sayfa|Kütüphane|Etkinlik|Ayarlar|Sınırsız Windows.|x86-64 yazılımlarını ARM64 Android’de çalıştırın.|Önizleme modu|Uygulama içe aktar|Çeviriyi dene|Son çalışma alanları|Tümünü gör|Araçlar|Dil|Uygulama dilini seçin|Dil ara|Geçerli|Görünüm|Prisma hakkında|Windows uygulamalarınız ve alanlarınız|Çeviri ve çalışma zamanı olayları|Henüz etkinlik yok|Prisma’yı kişiselleştirin|Yeni alan|Aç|Yakında|Motor hazır|uygulama"),
        "ar" to decodePack("الرئيسية|المكتبة|النشاط|الإعدادات|Windows بلا حدود.|شغّل برامج x86-64 على Android ARM64.|وضع المعاينة|استيراد تطبيق|جرّب الترجمة|مساحات العمل الأخيرة|عرض الكل|الأدوات|اللغة|اختر لغة التطبيق|البحث عن اللغات|الحالية|المظهر|حول Prisma|تطبيقات ومساحات Windows الخاصة بك|أحداث الترجمة والتشغيل|لا يوجد نشاط بعد|اجعل Prisma تناسبك|مساحة جديدة|فتح|قريبًا|المحرك جاهز|تطبيقات"),
        "he" to decodePack("בית|ספרייה|פעילות|הגדרות|Windows ללא גבולות.|הרצת תוכנות x86-64 ב-Android ARM64.|מצב תצוגה מקדימה|ייבוא יישום|ניסיון תרגום|סביבות עבודה אחרונות|הצגת הכול|כלים|שפה|בחירת שפת היישום|חיפוש שפות|נוכחית|מראה|אודות Prisma|יישומי וסביבות Windows שלך|אירועי תרגום והרצה|אין עדיין פעילות|התאמת Prisma עבורך|סביבה חדשה|פתיחה|בקרוב|המנוע מוכן|יישומים"),
        "fa" to decodePack("خانه|کتابخانه|فعالیت|تنظیمات|Windows بدون محدودیت.|نرم‌افزار x86-64 را در Android ARM64 اجرا کنید.|حالت پیش‌نمایش|وارد کردن برنامه|آزمایش ترجمه|فضاهای اخیر|نمایش همه|ابزارها|زبان|زبان برنامه را انتخاب کنید|جستجوی زبان‌ها|فعلی|ظاهر|درباره Prisma|برنامه‌ها و فضاهای Windows شما|رویدادهای ترجمه و اجرا|هنوز فعالیتی نیست|Prisma را شخصی کنید|فضای جدید|باز کردن|به‌زودی|موتور آماده است|برنامه‌ها"),
        "ur" to decodePack("ہوم|لائبریری|سرگرمی|ترتیبات|Windows، بغیر حدود کے۔|ARM64 Android پر x86-64 سافٹ ویئر چلائیں۔|پیش نظارہ موڈ|ایپ درآمد کریں|ترجمہ آزمائیں|حالیہ ورک اسپیس|سب دیکھیں|ٹولز|زبان|ایپ کی زبان منتخب کریں|زبانیں تلاش کریں|موجودہ|ظاہری شکل|Prisma کے بارے میں|آپ کی Windows ایپس اور ورک اسپیس|ترجمہ اور رن ٹائم واقعات|ابھی کوئی سرگرمی نہیں|Prisma کو اپنا بنائیں|نیا ورک اسپیس|کھولیں|جلد آرہا ہے|انجن تیار ہے|ایپس"),
        "hi" to decodePack("होम|लाइब्रेरी|गतिविधि|सेटिंग्स|Windows, बिना सीमाओं के।|ARM64 Android पर x86-64 सॉफ़्टवेयर चलाएँ।|पूर्वावलोकन मोड|ऐप आयात करें|अनुवाद आज़माएँ|हाल के कार्यस्थान|सभी देखें|उपकरण|भाषा|ऐप की भाषा चुनें|भाषाएँ खोजें|वर्तमान|रूप-रंग|Prisma के बारे में|आपके Windows ऐप और कार्यस्थान|अनुवाद और रनटाइम घटनाएँ|अभी कोई गतिविधि नहीं|Prisma को अपना बनाएँ|नया कार्यस्थान|खोलें|जल्द आ रहा है|इंजन तैयार है|ऐप"),
        "bn" to decodePack("হোম|লাইব্রেরি|কার্যকলাপ|সেটিংস|Windows, সীমাহীন।|ARM64 Android-এ x86-64 সফটওয়্যার চালান।|প্রিভিউ মোড|অ্যাপ আমদানি|অনুবাদ চেষ্টা করুন|সাম্প্রতিক কর্মক্ষেত্র|সব দেখুন|টুলস|ভাষা|অ্যাপের ভাষা বেছে নিন|ভাষা খুঁজুন|বর্তমান|চেহারা|Prisma সম্পর্কে|আপনার Windows অ্যাপ ও কর্মক্ষেত্র|অনুবাদ ও রানটাইম ঘটনা|এখনও কোনো কার্যকলাপ নেই|Prisma-কে নিজের মতো করুন|নতুন কর্মক্ষেত্র|খুলুন|শীঘ্রই আসছে|ইঞ্জিন প্রস্তুত|অ্যাপ"),
        "ta" to decodePack("முகப்பு|நூலகம்|செயல்பாடு|அமைப்புகள்|Windows, வரம்புகள் இன்றி.|ARM64 Android-ல் x86-64 மென்பொருளை இயக்குங்கள்.|முன்னோட்ட முறை|செயலியை இறக்குமதி செய்|மொழிபெயர்ப்பை முயற்சி செய்|சமீபத்திய பணியிடங்கள்|அனைத்தையும் காண்|கருவிகள்|மொழி|செயலியின் மொழியைத் தேர்ந்தெடுக்கவும்|மொழிகளைத் தேடு|தற்போதைய|தோற்றம்|Prisma பற்றி|உங்கள் Windows செயலிகளும் பணியிடங்களும்|மொழிபெயர்ப்பு மற்றும் இயக்க நிகழ்வுகள்|இன்னும் செயல்பாடு இல்லை|Prisma-வை உங்களுடையதாக்குங்கள்|புதிய பணியிடம்|திற|விரைவில்|இயந்திரம் தயார்|செயலிகள்"),
        "te" to decodePack("హోమ్|లైబ్రరీ|కార్యాచరణ|సెట్టింగ్‌లు|Windows, పరిమితులు లేకుండా.|ARM64 Androidలో x86-64 సాఫ్ట్‌వేర్‌ను నడపండి.|ప్రివ్యూ మోడ్|యాప్ దిగుమతి|అనువాదాన్ని ప్రయత్నించండి|ఇటీవలి వర్క్‌స్పేస్‌లు|అన్నీ చూడండి|సాధనాలు|భాష|యాప్ భాషను ఎంచుకోండి|భాషలను వెతకండి|ప్రస్తుత|రూపం|Prisma గురించి|మీ Windows యాప్‌లు మరియు వర్క్‌స్పేస్‌లు|అనువాద మరియు రన్‌టైమ్ ఈవెంట్‌లు|ఇంకా కార్యాచరణ లేదు|Prismaను మీకు అనుగుణంగా చేసుకోండి|కొత్త వర్క్‌స్పేస్|తెరవండి|త్వరలో|ఇంజిన్ సిద్ధంగా ఉంది|యాప్‌లు"),
        "mr" to decodePack("मुख्यपृष्ठ|लायब्ररी|क्रियाकलाप|सेटिंग्ज|Windows, मर्यादांशिवाय.|ARM64 Android वर x86-64 सॉफ्टवेअर चालवा.|पूर्वावलोकन मोड|ॲप आयात करा|भाषांतर वापरून पाहा|अलीकडील कार्यक्षेत्रे|सर्व पहा|साधने|भाषा|ॲपची भाषा निवडा|भाषा शोधा|सध्याची|स्वरूप|Prisma विषयी|तुमची Windows ॲप्स आणि कार्यक्षेत्रे|भाषांतर आणि रनटाइम घटना|अद्याप कोणतीही क्रिया नाही|Prisma तुमचे बनवा|नवीन कार्यक्षेत्र|उघडा|लवकरच|इंजिन तयार|ॲप्स"),
        "gu" to decodePack("હોમ|લાઇબ્રેરી|પ્રવૃત્તિ|સેટિંગ્સ|Windows, મર્યાદા વિના.|ARM64 Android પર x86-64 સોફ્ટવેર ચલાવો.|પૂર્વાવલોકન મોડ|એપ આયાત કરો|અનુવાદ અજમાવો|તાજેતરના વર્કસ્પેસ|બધું જુઓ|સાધનો|ભાષા|એપની ભાષા પસંદ કરો|ભાષાઓ શોધો|વર્તમાન|દેખાવ|Prisma વિશે|તમારી Windows એપ્સ અને વર્કસ્પેસ|અનુવાદ અને રનટાઇમ ઘટનાઓ|હજી કોઈ પ્રવૃત્તિ નથી|Prismaને તમારું બનાવો|નવું વર્કસ્પેસ|ખોલો|ટૂંક સમયમાં|એન્જિન તૈયાર|એપ્સ"),
        "pa" to decodePack("ਮੁੱਖ|ਲਾਇਬ੍ਰੇਰੀ|ਸਰਗਰਮੀ|ਸੈਟਿੰਗਾਂ|Windows, ਬਿਨਾਂ ਹੱਦਾਂ ਦੇ।|ARM64 Android ਉੱਤੇ x86-64 ਸਾਫਟਵੇਅਰ ਚਲਾਓ।|ਝਲਕ ਮੋਡ|ਐਪ ਆਯਾਤ ਕਰੋ|ਅਨੁਵਾਦ ਅਜ਼ਮਾਓ|ਹਾਲੀਆ ਵਰਕਸਪੇਸ|ਸਭ ਦੇਖੋ|ਟੂਲ|ਭਾਸ਼ਾ|ਐਪ ਦੀ ਭਾਸ਼ਾ ਚੁਣੋ|ਭਾਸ਼ਾਵਾਂ ਖੋਜੋ|ਮੌਜੂਦਾ|ਦਿੱਖ|Prisma ਬਾਰੇ|ਤੁਹਾਡੀਆਂ Windows ਐਪਾਂ ਅਤੇ ਵਰਕਸਪੇਸ|ਅਨੁਵਾਦ ਅਤੇ ਰਨਟਾਈਮ ਘਟਨਾਵਾਂ|ਹਾਲੇ ਕੋਈ ਸਰਗਰਮੀ ਨਹੀਂ|Prisma ਨੂੰ ਆਪਣਾ ਬਣਾਓ|ਨਵਾਂ ਵਰਕਸਪੇਸ|ਖੋਲ੍ਹੋ|ਜਲਦੀ ਆ ਰਿਹਾ ਹੈ|ਇੰਜਣ ਤਿਆਰ ਹੈ|ਐਪਾਂ"),
        "id" to decodePack("Beranda|Pustaka|Aktivitas|Pengaturan|Windows, tanpa batas.|Jalankan perangkat lunak x86-64 di Android ARM64.|Mode pratinjau|Impor aplikasi|Coba translasi|Ruang kerja terbaru|Lihat semua|Alat|Bahasa|Pilih bahasa aplikasi|Cari bahasa|Saat ini|Tampilan|Tentang Prisma|Aplikasi dan ruang kerja Windows Anda|Peristiwa translasi dan runtime|Belum ada aktivitas|Jadikan Prisma milik Anda|Ruang kerja baru|Buka|Segera hadir|Mesin siap|aplikasi"),
        "ms" to decodePack("Laman utama|Pustaka|Aktiviti|Tetapan|Windows, tanpa had.|Jalankan perisian x86-64 pada Android ARM64.|Mod pratonton|Import aplikasi|Cuba terjemahan|Ruang kerja terkini|Lihat semua|Alat|Bahasa|Pilih bahasa aplikasi|Cari bahasa|Semasa|Penampilan|Tentang Prisma|Aplikasi dan ruang kerja Windows anda|Peristiwa terjemahan dan masa jalan|Belum ada aktiviti|Jadikan Prisma milik anda|Ruang kerja baharu|Buka|Akan datang|Enjin sedia|aplikasi"),
        "vi" to decodePack("Trang chủ|Thư viện|Hoạt động|Cài đặt|Windows, không giới hạn.|Chạy phần mềm x86-64 trên Android ARM64.|Chế độ xem trước|Nhập ứng dụng|Thử dịch mã|Không gian gần đây|Xem tất cả|Công cụ|Ngôn ngữ|Chọn ngôn ngữ ứng dụng|Tìm ngôn ngữ|Hiện tại|Giao diện|Giới thiệu Prisma|Ứng dụng và không gian Windows của bạn|Sự kiện dịch mã và thực thi|Chưa có hoạt động|Biến Prisma thành của bạn|Không gian mới|Mở|Sắp ra mắt|Bộ máy sẵn sàng|ứng dụng"),
        "th" to decodePack("หน้าหลัก|คลัง|กิจกรรม|การตั้งค่า|Windows แบบไร้ขีดจำกัด|เรียกใช้ซอฟต์แวร์ x86-64 บน Android ARM64|โหมดแสดงตัวอย่าง|นำเข้าแอป|ลองแปลคำสั่ง|พื้นที่ล่าสุด|ดูทั้งหมด|เครื่องมือ|ภาษา|เลือกภาษาของแอป|ค้นหาภาษา|ปัจจุบัน|รูปลักษณ์|เกี่ยวกับ Prisma|แอปและพื้นที่ Windows ของคุณ|เหตุการณ์การแปลและรันไทม์|ยังไม่มีกิจกรรม|ปรับ Prisma ให้เป็นของคุณ|พื้นที่ใหม่|เปิด|เร็ว ๆ นี้|เอนจินพร้อม|แอป"),
        "fil" to decodePack("Home|Library|Aktibidad|Mga Setting|Windows, walang limitasyon.|Patakbuhin ang x86-64 software sa ARM64 Android.|Preview mode|Mag-import ng app|Subukan ang translation|Mga kamakailang workspace|Tingnan lahat|Mga tool|Wika|Piliin ang wika ng app|Maghanap ng wika|Kasalukuyan|Hitsura|Tungkol sa Prisma|Iyong Windows apps at workspace|Mga event ng translation at runtime|Wala pang aktibidad|Gawing iyo ang Prisma|Bagong workspace|Buksan|Malapit na|Handa ang engine|mga app"),
        "zh-CN" to decodePack("首页|应用库|活动|设置|Windows，不设限。|在 ARM64 Android 上运行 x86-64 软件。|预览模式|导入应用|试用转译|最近的工作区|查看全部|工具|语言|选择应用语言|搜索语言|当前|外观|关于 Prisma|你的 Windows 应用和工作区|转译与运行时事件|暂无活动|打造你的 Prisma|新建工作区|打开|即将推出|引擎已就绪|应用"),
        "zh-TW" to decodePack("首頁|應用程式庫|活動|設定|Windows，不設限。|在 ARM64 Android 上執行 x86-64 軟體。|預覽模式|匯入應用程式|試用轉譯|最近的工作區|查看全部|工具|語言|選擇應用程式語言|搜尋語言|目前|外觀|關於 Prisma|你的 Windows 應用程式與工作區|轉譯與執行階段事件|尚無活動|打造你的 Prisma|新增工作區|開啟|即將推出|引擎已就緒|應用程式"),
        "ja" to decodePack("ホーム|ライブラリ|アクティビティ|設定|Windowsを、限界なく。|ARM64 Androidでx86-64ソフトウェアを実行。|プレビューモード|アプリをインポート|変換を試す|最近のワークスペース|すべて表示|ツール|言語|アプリの言語を選択|言語を検索|現在|外観|Prismaについて|Windowsアプリとワークスペース|変換とランタイムのイベント|アクティビティはまだありません|Prismaを自分好みに|新しいワークスペース|開く|近日公開|エンジン準備完了|アプリ"),
        "ko" to decodePack("홈|라이브러리|활동|설정|한계 없는 Windows.|ARM64 Android에서 x86-64 소프트웨어를 실행하세요.|미리보기 모드|앱 가져오기|변환 체험|최근 작업 공간|모두 보기|도구|언어|앱 언어 선택|언어 검색|현재|화면 설정|Prisma 정보|Windows 앱과 작업 공간|변환 및 런타임 이벤트|아직 활동이 없습니다|Prisma를 나만의 방식으로|새 작업 공간|열기|출시 예정|엔진 준비됨|앱"),
        "sv" to decodePack("Hem|Bibliotek|Aktivitet|Inställningar|Windows, utan gränser.|Kör x86-64-programvara på ARM64 Android.|Förhandsvisning|Importera app|Prova översättning|Senaste arbetsytor|Visa alla|Verktyg|Språk|Välj appens språk|Sök språk|Aktuellt|Utseende|Om Prisma|Dina Windows-appar och arbetsytor|Översättnings- och runtimehändelser|Ingen aktivitet ännu|Gör Prisma till ditt|Ny arbetsyta|Öppna|Kommer snart|Motorn är redo|appar"),
        "no" to decodePack("Hjem|Bibliotek|Aktivitet|Innstillinger|Windows, uten grenser.|Kjør x86-64-programvare på ARM64 Android.|Forhåndsvisning|Importer app|Prøv oversettelse|Nylige arbeidsområder|Vis alle|Verktøy|Språk|Velg appens språk|Søk etter språk|Gjeldende|Utseende|Om Prisma|Windows-appene og arbeidsområdene dine|Oversettelses- og kjøretidshendelser|Ingen aktivitet ennå|Gjør Prisma til din|Nytt arbeidsområde|Åpne|Kommer snart|Motoren er klar|apper"),
        "da" to decodePack("Hjem|Bibliotek|Aktivitet|Indstillinger|Windows, uden grænser.|Kør x86-64-software på ARM64 Android.|Forhåndsvisning|Importér app|Prøv oversættelse|Seneste arbejdsområder|Vis alle|Værktøjer|Sprog|Vælg appens sprog|Søg efter sprog|Aktuelt|Udseende|Om Prisma|Dine Windows-apps og arbejdsområder|Oversættelses- og runtimehændelser|Ingen aktivitet endnu|Gør Prisma til din|Nyt arbejdsområde|Åbn|Kommer snart|Motoren er klar|apps"),
        "fi" to decodePack("Koti|Kirjasto|Toiminta|Asetukset|Windows ilman rajoja.|Suorita x86-64-ohjelmistoja ARM64 Androidilla.|Esikatselutila|Tuo sovellus|Kokeile käännöstä|Viimeisimmät työtilat|Näytä kaikki|Työkalut|Kieli|Valitse sovelluksen kieli|Hae kieliä|Nykyinen|Ulkoasu|Tietoja Prismasta|Windows-sovelluksesi ja työtilasi|Käännös- ja suoritustapahtumat|Ei vielä toimintaa|Tee Prismasta omasi|Uusi työtila|Avaa|Tulossa pian|Moottori valmis|sovellusta"),
        "et" to decodePack("Avaleht|Teek|Tegevus|Seaded|Windows ilma piirideta.|Käivita x86-64 tarkvara ARM64 Androidis.|Eelvaaterežiim|Impordi rakendus|Proovi tõlget|Hiljutised tööruumid|Vaata kõiki|Tööriistad|Keel|Vali rakenduse keel|Otsi keeli|Praegune|Välimus|Prisma teave|Sinu Windowsi rakendused ja tööruumid|Tõlke- ja käitussündmused|Tegevusi pole veel|Tee Prisma enda omaks|Uus tööruum|Ava|Peagi tulekul|Mootor on valmis|rakendust"),
        "lv" to decodePack("Sākums|Bibliotēka|Aktivitāte|Iestatījumi|Windows bez robežām.|Palaidiet x86-64 programmatūru ARM64 Android ierīcē.|Priekšskatījuma režīms|Importēt lietotni|Izmēģināt tulkošanu|Nesenās darbvietas|Skatīt visu|Rīki|Valoda|Izvēlieties lietotnes valodu|Meklēt valodas|Pašreizējā|Izskats|Par Prisma|Jūsu Windows lietotnes un darbvietas|Tulkošanas un izpildes notikumi|Vēl nav aktivitātes|Pielāgojiet Prisma|Jauna darbvieta|Atvērt|Drīzumā|Dzinējs gatavs|lietotnes"),
        "lt" to decodePack("Pradžia|Biblioteka|Veikla|Nustatymai|Windows be ribų.|Paleiskite x86-64 programinę įrangą ARM64 Android įrenginyje.|Peržiūros režimas|Importuoti programą|Išbandyti vertimą|Naujausios darbo erdvės|Rodyti visas|Įrankiai|Kalba|Pasirinkite programos kalbą|Ieškoti kalbų|Dabartinė|Išvaizda|Apie Prisma|Jūsų Windows programos ir darbo erdvės|Vertimo ir vykdymo įvykiai|Veiklos dar nėra|Pritaikykite Prisma sau|Nauja darbo erdvė|Atidaryti|Netrukus|Variklis paruoštas|programos"),
        "sl" to decodePack("Domov|Knjižnica|Dejavnost|Nastavitve|Windows brez omejitev.|Zaženite programsko opremo x86-64 v sistemu ARM64 Android.|Način predogleda|Uvozi aplikacijo|Preizkusi prevajanje|Nedavni delovni prostori|Prikaži vse|Orodja|Jezik|Izberite jezik aplikacije|Išči jezike|Trenutni|Videz|O Prismu|Vaše aplikacije in prostori Windows|Dogodki prevajanja in izvajanja|Ni še nobene dejavnosti|Prilagodite Prisma|Nov delovni prostor|Odpri|Kmalu|Pogon je pripravljen|aplikacije"),
        "hr" to decodePack("Početna|Biblioteka|Aktivnost|Postavke|Windows bez granica.|Pokrenite x86-64 softver na ARM64 Androidu.|Način pregleda|Uvezi aplikaciju|Isprobaj prevođenje|Nedavni radni prostori|Prikaži sve|Alati|Jezik|Odaberite jezik aplikacije|Pretraži jezike|Trenutni|Izgled|O Prismi|Vaše Windows aplikacije i prostori|Događaji prevođenja i izvođenja|Još nema aktivnosti|Prilagodite Prismu|Novi radni prostor|Otvori|Uskoro|Pogon spreman|aplikacije"),
        "sr-Latn" to decodePack("Početna|Biblioteka|Aktivnost|Podešavanja|Windows bez granica.|Pokrenite x86-64 softver na ARM64 Androidu.|Režim pregleda|Uvezi aplikaciju|Probaj prevođenje|Nedavni radni prostori|Prikaži sve|Alati|Jezik|Izaberite jezik aplikacije|Pretraži jezike|Trenutni|Izgled|O Prismi|Vaše Windows aplikacije i prostori|Događaji prevođenja i izvršavanja|Još nema aktivnosti|Prilagodite Prismu|Novi radni prostor|Otvori|Uskoro|Pogon je spreman|aplikacije"),
        "bs" to decodePack("Početna|Biblioteka|Aktivnost|Postavke|Windows bez granica.|Pokrenite x86-64 softver na ARM64 Androidu.|Način pregleda|Uvezi aplikaciju|Isprobaj prevođenje|Nedavni radni prostori|Prikaži sve|Alati|Jezik|Odaberite jezik aplikacije|Pretraži jezike|Trenutni|Izgled|O Prismi|Vaše Windows aplikacije i prostori|Događaji prevođenja i izvršavanja|Još nema aktivnosti|Prilagodite Prismu|Novi radni prostor|Otvori|Uskoro|Pogon spreman|aplikacije"),
        "mk" to decodePack("Почетна|Библиотека|Активност|Поставки|Windows без граници.|Стартувајте x86-64 софтвер на ARM64 Android.|Режим за преглед|Увези апликација|Пробај превод|Неодамнешни простори|Прикажи ги сите|Алатки|Јазик|Изберете јазик на апликацијата|Пребарај јазици|Тековен|Изглед|За Prisma|Вашите Windows апликации и простори|Настани за превод и извршување|Сè уште нема активност|Приспособете ја Prisma|Нов простор|Отвори|Наскоро|Моторот е подготвен|апликации"),
        "sq" to decodePack("Kreu|Biblioteka|Aktiviteti|Cilësimet|Windows, pa kufij.|Ekzekutoni programe x86-64 në Android ARM64.|Modaliteti i pamjes|Importo aplikacion|Provo përkthimin|Hapësirat e fundit|Shiko të gjitha|Mjetet|Gjuha|Zgjidhni gjuhën e aplikacionit|Kërko gjuhë|Aktuale|Pamja|Rreth Prisma|Aplikacionet dhe hapësirat tuaja Windows|Ngjarjet e përkthimit dhe ekzekutimit|Ende pa aktivitet|Bëjeni Prisma tuajën|Hapësirë e re|Hap|Së shpejti|Motori gati|aplikacione"),
        "sw" to decodePack("Nyumbani|Maktaba|Shughuli|Mipangilio|Windows bila mipaka.|Endesha programu za x86-64 kwenye Android ARM64.|Hali ya hakikisho|Leta programu|Jaribu tafsiri|Nafasi za hivi karibuni|Tazama zote|Zana|Lugha|Chagua lugha ya programu|Tafuta lugha|Ya sasa|Mwonekano|Kuhusu Prisma|Programu na nafasi zako za Windows|Matukio ya tafsiri na utekelezaji|Bado hakuna shughuli|Fanya Prisma iwe yako|Nafasi mpya|Fungua|Inakuja hivi karibuni|Injini iko tayari|programu"),
        "af" to decodePack("Tuis|Biblioteek|Aktiwiteit|Instellings|Windows, sonder perke.|Loop x86-64-sagteware op ARM64 Android.|Voorskoumodus|Voer app in|Probeer vertaling|Onlangse werkruimtes|Wys alles|Gereedskap|Taal|Kies die app se taal|Soek tale|Huidig|Voorkoms|Oor Prisma|Jou Windows-apps en werkruimtes|Vertaal- en looptydgebeure|Nog geen aktiwiteit nie|Maak Prisma jou eie|Nuwe werkruimte|Maak oop|Kom binnekort|Enjin gereed|apps"),
    )

    fun language(tag: String): AppLanguage =
        supported.firstOrNull { it.tag == tag } ?: supported.first()

    fun pack(tag: String): LanguagePack = packs[tag] ?: checkNotNull(packs["en"])

    fun validate() {
        require(supported.size > 50)
        require(supported.map { it.tag }.toSet().size == supported.size)
        require(supported.all { it.tag in packs })
    }
}

class LanguagePreferences(context: Context) {
    private val preferences = context.getSharedPreferences("prisma-ui", Context.MODE_PRIVATE)

    fun load(): String = preferences.getString("language", "en") ?: "en"

    fun save(tag: String) {
        require(PrismaLanguages.supported.any { it.tag == tag })
        preferences.edit().putString("language", tag).apply()
    }
}

val LocalPrismaLanguage = staticCompositionLocalOf { PrismaLanguages.language("en") }
val LocalPrismaStrings = staticCompositionLocalOf { PrismaLanguages.pack("en") }

@Composable
fun PrismaLocale(languageTag: String, content: @Composable () -> Unit) {
    val language = PrismaLanguages.language(languageTag)
    CompositionLocalProvider(
        LocalPrismaLanguage provides language,
        LocalPrismaStrings provides PrismaLanguages.pack(language.tag),
        LocalLayoutDirection provides if (language.isRtl) LayoutDirection.Rtl else LayoutDirection.Ltr,
        content = content,
    )
}

@Composable
fun tr(key: UiText): String = LocalPrismaStrings.current[key]

data class TechnicalCopy(
    val back: String,
    val cancel: String,
    val close: String,
    val create: String,
    val name: String,
    val importedInto: String,
    val importFailed: String,
    val executedSuccessfully: String,
    val executionFailed: String,
    val arm64Required: String,
    val translationInspector: String,
    val developerTerminal: String,
    val steamLibrary: String,
    val inputMapper: String,
    val winePrefixesDetail: String,
    val guestProcessIo: String,
    val libraryDiscovery: String,
    val touchBindings: String,
    val activityEmptyDetail: String,
    val terminalSubtitle: String,
    val clear: String,
    val enterCommand: String,
    val shellPreview: String,
    val waitingBridge: String,
    val bridgeUnavailable: String,
    val translatingSample: String,
    val traceComplete: String,
    val replay: String,
    val preview: String,
    val translationTrace: String,
    val elapsedResult: String,
    val previewOnly: String,
    val inspectorSampleNote: String,
    val sampleViewport: String,
    val sampleFrame: String,
    val guest: String,
    val hostTarget: String,
    val block: String,
    val cache: String,
    val working: String,
)

object PrismaTechnicalCopies {
    private val english = TechnicalCopy(
        back = "Back",
        cancel = "Cancel",
        close = "Close",
        create = "Create",
        name = "Name",
        importedInto = "Imported into",
        importFailed = "Import failed",
        executedSuccessfully = "Executed successfully in Rust.",
        executionFailed = "Execution failed in Rust",
        arm64Required = "The Rust engine requires the ARM64 device build.",
        translationInspector = "Translation inspector",
        developerTerminal = "Developer terminal",
        steamLibrary = "Steam library",
        inputMapper = "Input mapper",
        winePrefixesDetail = "Wine prefixes · isolated by default",
        guestProcessIo = "Guest process I/O",
        libraryDiscovery = "Library discovery will connect games to isolated Prisma workspaces.",
        touchBindings = "Touch zones and XInput bindings will be saved per application profile.",
        activityEmptyDetail = "Decode, cache and guest process events will appear here.",
        terminalSubtitle = "x86-64 preview · bridge disconnected",
        clear = "Clear",
        enterCommand = "Enter command",
        shellPreview = "Prisma shell preview",
        waitingBridge = "Waiting for the Rust terminal bridge…",
        bridgeUnavailable = "Bridge unavailable: this AVD is x86-64.",
        translatingSample = "Translating sample block",
        traceComplete = "Trace complete · 0.72 ms",
        replay = "Replay",
        preview = "Preview",
        translationTrace = "Translation trace",
        elapsedResult = "elapsed / result",
        previewOnly = "Preview only. This x86-64 emulator can inspect ARM64 output but cannot execute it.",
        inspectorSampleNote = "Inspector data is deterministic sample output. Native execution remains disabled on x86-64.",
        sampleViewport = "Sample render viewport",
        sampleFrame = "sample frame · UI preview",
        guest = "Guest",
        hostTarget = "Host target",
        block = "Block",
        cache = "Cache",
        working = "working",
    )

    private val localized = mapOf(
        "es" to english.copy(
            back = "Volver",
            cancel = "Cancelar",
            close = "Cerrar",
            create = "Crear",
            name = "Nombre",
            importedInto = "Importado en",
            importFailed = "Error al importar",
            executedSuccessfully = "Ejecutado correctamente en Rust.",
            executionFailed = "La ejecución en Rust falló",
            arm64Required = "El motor Rust requiere la compilación para un dispositivo ARM64.",
            translationInspector = "Inspector de traducción",
            developerTerminal = "Terminal de desarrollo",
            steamLibrary = "Biblioteca de Steam",
            inputMapper = "Mapeo de controles",
            winePrefixesDetail = "Prefijos Wine · aislados por defecto",
            guestProcessIo = "E/S del proceso invitado",
            libraryDiscovery = "La biblioteca conectará los juegos con espacios aislados de Prisma.",
            touchBindings = "Las zonas táctiles y controles XInput se guardarán por perfil.",
            activityEmptyDetail = "Aquí aparecerán los eventos de decodificación, caché y procesos.",
            terminalSubtitle = "vista previa x86-64 · puente desconectado",
            clear = "Limpiar",
            enterCommand = "Escribe un comando",
            shellPreview = "Vista previa de la consola Prisma",
            waitingBridge = "Esperando el puente de terminal de Rust…",
            bridgeUnavailable = "Puente no disponible: este AVD es x86-64.",
            translatingSample = "Traduciendo bloque de muestra",
            traceComplete = "Traza completa · 0.72 ms",
            replay = "Repetir",
            preview = "Vista previa",
            translationTrace = "Traza de traducción",
            elapsedResult = "tiempo / resultado",
            previewOnly = "Solo vista previa. Este emulador x86-64 puede inspeccionar la salida ARM64, pero no ejecutarla.",
            inspectorSampleNote = "El inspector usa una muestra determinista. La ejecución nativa sigue desactivada en x86-64.",
            sampleViewport = "Vista previa del render",
            sampleFrame = "fotograma de muestra · vista previa",
            guest = "Invitado",
            hostTarget = "Destino host",
            block = "Bloque",
            cache = "Caché",
            working = "procesando",
        ),
        "ar" to english.copy(
            back = "رجوع",
            cancel = "إلغاء",
            close = "إغلاق",
            create = "إنشاء",
            name = "الاسم",
            importedInto = "تم الاستيراد إلى",
            importFailed = "فشل الاستيراد",
            executedSuccessfully = "تم التنفيذ بنجاح في Rust.",
            executionFailed = "فشل التنفيذ في Rust",
            arm64Required = "يتطلب محرك Rust إصدار الجهاز ARM64.",
            translationInspector = "فاحص الترجمة",
            developerTerminal = "طرفية المطور",
            steamLibrary = "مكتبة Steam",
            inputMapper = "تعيين وحدات التحكم",
            winePrefixesDetail = "بادئات Wine · معزولة افتراضيًا",
            guestProcessIo = "إدخال وإخراج عملية الضيف",
            libraryDiscovery = "ستربط المكتبة الألعاب بمساحات Prisma المعزولة.",
            touchBindings = "ستُحفظ مناطق اللمس وروابط XInput لكل ملف تطبيق.",
            activityEmptyDetail = "ستظهر هنا أحداث فك الترميز والتخزين المؤقت وعمليات الضيف.",
            terminalSubtitle = "معاينة x86-64 · الجسر غير متصل",
            clear = "مسح",
            enterCommand = "أدخل أمرًا",
            shellPreview = "معاينة طرفية Prisma",
            waitingBridge = "في انتظار جسر طرفية Rust…",
            bridgeUnavailable = "الجسر غير متاح: جهاز AVD هذا يعمل بمعمارية x86-64.",
            translatingSample = "جارٍ ترجمة كتلة نموذجية",
            traceComplete = "اكتمل التتبع · 0.72 ms",
            replay = "إعادة",
            preview = "معاينة",
            translationTrace = "تتبع الترجمة",
            elapsedResult = "الوقت / النتيجة",
            previewOnly = "معاينة فقط. يمكن لهذا المحاكي x86-64 فحص خرج ARM64 لكنه لا يستطيع تنفيذه.",
            inspectorSampleNote = "بيانات الفاحص نموذج حتمي. يظل التنفيذ الأصلي معطلاً على x86-64.",
            sampleViewport = "منطقة عرض نموذجية",
            sampleFrame = "إطار نموذجي · معاينة الواجهة",
            guest = "الضيف",
            hostTarget = "هدف المضيف",
            block = "الكتلة",
            cache = "التخزين المؤقت",
            working = "قيد العمل",
        ),
    )

    fun forTag(tag: String): TechnicalCopy = localized[tag] ?: english
}

@Composable
fun technicalCopy(): TechnicalCopy = PrismaTechnicalCopies.forTag(LocalPrismaLanguage.current.tag)
