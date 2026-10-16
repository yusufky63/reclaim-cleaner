# FAQ

[Türkçe sorular aşağıda](#sss)

### Is it safe?

ReClaim only deletes what you confirmed in a preview. Items it is not sure about are marked "Review first" and are not preselected; risky ones stay hidden. System folders, your profile and libraries, OneDrive, `.git` folders and anything you exclude in Settings can never be removed. Links are never followed. See [the safety model](architecture.md#safety-model).

### Why are caches deleted permanently instead of going to the Recycle Bin?

Caches rebuild themselves, and moving gigabytes of cache into the Recycle Bin frees nothing until you empty it. Your own files (large files, duplicates, empty folders, old installers) go to the Recycle Bin by default. Both choices are in Settings and every confirm dialog says which one is used.

### Why does it ask for administrator rights?

Only for items under Windows-owned folders, such as `C:\Windows\Temp` or the Windows Update download cache. The main app stays a normal user process; one UAC prompt starts a short-lived helper for those items. You can also restart the whole app as administrator from Settings.

### The numbers do not match Explorer.

ReClaim counts file sizes, not allocated clusters, and skips online-only OneDrive files because they take no local space. Folders you cannot read are listed as "Not scanned". Hard-linked files (common in `C:\Windows\WinSxS`) can be counted more than once, as Explorer also does.

### Why does C: use more than its folders add up to?

Windows keeps space that no folder scan can open or see: restore points and shadow copies in System Volume Information, reserved storage for updates, the NTFS file table, and the page and hibernation files. Map the whole drive on the Space map and the Hidden space panel lists each part, says whether the map counted it, and links to the Windows screen that manages it (System Protection for restore points, Performance Options for the page file). Windows shows restore point sizes only to administrators, so ReClaim asks once to measure them and only reads. What is left is marked "Not explained": small files take up a whole disk cluster and hard links are counted more than once, so a map and a drive never match exactly.

### How do I shrink a WSL or Docker disk?

`ext4.vhdx` and `docker_data.vhdx` grow as Linux writes data and keep their size after files inside are deleted. ReClaim shows them but does not change them. Back the disk up first, then:

1. Remove what you do not need inside Linux (for Docker, `docker system prune`).
2. Run `wsl --shutdown`.
3. In an administrator command prompt, run `diskpart`, then `select vdisk file="<path to the .vhdx>"`, `attach vdisk readonly`, `compact vdisk`, `detach vdisk` and `exit`. With Hyper-V installed, `Optimize-VHD -Path <path> -Mode Full` does the same.

Recent WSL versions can keep a disk sparse so that it shrinks on its own: `wsl --manage <distro> --set-sparse true`.

### How do I add ReClaim to Explorer's right-click menu?

Settings > Explorer integration > "Add Analyze with ReClaim to the right-click menu". The entry appears on folders, folder backgrounds and drives for your Windows user only, and needs no administrator rights. On Windows 11 it is under "Show more options" (or Shift+F10): the new top-level menu only accepts handlers from signed app packages, which ReClaim does not ship yet. Selecting several folders and choosing the entry maps them together.

To remove it, turn the switch off. The setup (`.exe`) removes it when you uninstall, but keeps it during updates. The MSI does not remove per-user entries (it installs for all users, the entry belongs to one), so turn the switch off before uninstalling an MSI install; a leftover entry only fails to start the missing program. For the portable build, turn it off before deleting the folder. The keys are `HKCU\Software\Classes\Directory\shell\ReClaim.Analyze`, `...\Directory\Background\shell\ReClaim.Analyze` and `...\Drive\shell\ReClaim.Analyze`.

### A file cannot be deleted because it is in use. What is using it?

Choose "What is using this?" in Large files, on the Space map, or next to an "In use" line after a clean. ReClaim lists the apps, background programs, services and Windows processes that hold the file, using the Restart Manager installers rely on. For a folder it also finds programs that run from it or have it open as their working folder, such as a terminal. "Close app" asks the app to close the way its close button does, so it can still ask you to save; services and Windows processes are never stopped from here, and ReClaim never ends processes or forces files closed.

### My Steam games are on another drive. Does ReClaim find them?

Yes. The Games page and the Steam cleanup rules read Steam's own library list (`libraryfolders.vdf`), so every library you added in Steam > Settings > Storage is included, on any drive, along with the Steam folder itself, wherever it is installed. A folder that was moved by hand and linked back with a junction is not followed, because ReClaim never follows links; add it to Steam as a library instead.

### Why is my browser cache marked "Close Chrome to clean"?

Browsers keep their cache files open. Cleaning while the browser runs would fail halfway or corrupt the cache, so the rule waits until the browser is closed.

### What does the leftover scan after an uninstall remove?

Nothing on its own. After the app's own uninstaller has finished, ReClaim lists folders in AppData, ProgramData and Program Files that carry the app's or the publisher's name. The install folder, folders named exactly like the app and `Publisher\App` folders are high confidence and preselected. A publisher folder on its own or a similar name is low confidence and stays unchecked, because it can hold settings, game saves or files of other apps. Folders another installed app may still use are not listed. Removal goes to the Recycle Bin by default; folders in Program Files and ProgramData need administrator rights.

### Does it clean the registry?

No. Microsoft does not support registry cleaners and the space gain is negligible.

### Does it send any data?

No. There is no telemetry or account. The optional update check sends one request to the GitHub API.

### Where are settings and history stored?

`%APPDATA%\ReClaim`, together with your own rules (`rules`) and the `.reg` backups of removed uninstall entries (`backups`). In the portable build, in the `data` folder next to the exe.

Uninstalling the setup (`.exe`) version deletes this folder only when you tick "Delete the application data". A new setup that offers to uninstall the old version first shows the same box; leave it unticked to keep your settings. The MSI never deletes it.

### How do I add a location ReClaim does not know?

Open a "Cleanup rule request" issue, or write a rule yourself: [docs/rules.md](rules.md).

---

## SSS

### Güvenli mi?

ReClaim sadece önizlemede onayladığın şeyleri siler. Emin olmadığı öğeler "Önce gözden geçir" olarak işaretlenir ve önceden seçilmez; riskli olanlar gizli kalır. Sistem klasörleri, kullanıcı ve kitaplık klasörlerin, OneDrive, `.git` klasörleri ve Ayarlar'da hariç tuttuğun her şey hiçbir şekilde silinemez. Bağlantılar asla izlenmez.

### Önbellekler neden Geri Dönüşüm Kutusu'na değil de kalıcı siliniyor?

Önbellekler kendini yeniden kurar; gigabaytlarca önbelleği Geri Dönüşüm Kutusu'na taşımak, kutuyu boşaltana kadar alan açmaz. Senin dosyaların (büyük dosyalar, kopyalar, boş klasörler, eski kurulum dosyaları) varsayılan olarak Geri Dönüşüm Kutusu'na gider. İkisi de Ayarlar'dan değiştirilebilir.

### Neden yönetici izni istiyor?

Sadece `C:\Windows\Temp` ya da Windows Update indirme önbelleği gibi Windows'a ait klasörlerdeki öğeler için. Ana uygulama normal kullanıcı olarak kalır; tek bir UAC onayı bu öğeler için kısa ömürlü bir yardımcı başlatır.

### Sayılar Gezgin'le tutmuyor.

ReClaim dosya boyutlarını sayar, sadece bulutta duran OneDrive dosyalarını yerel alan kaplamadıkları için atlar. Okunamayan klasörler "Taranmadı" olarak gösterilir.

### Kaldırma sonrası artık taraması neyi siler?

Kendi başına hiçbir şeyi. Uygulamanın kendi kaldırıcısı bittikten sonra ReClaim, AppData, ProgramData ve Program Files içinde uygulamanın ya da yayıncının adını taşıyan klasörleri listeler. Kurulum klasörü, uygulamayla aynı adı taşıyan klasörler ve `Yayıncı\Uygulama` klasörleri yüksek güvenlidir ve önceden seçilir. Tek başına yayıncı klasörü ya da benzer bir ad düşük güvenlidir ve seçilmez, çünkü ayarlar, oyun kayıtları ya da başka uygulamaların dosyaları olabilir. Kurulu başka bir uygulamanın kullanıyor olabileceği klasörler listelenmez. Varsayılan olarak Geri Dönüşüm Kutusu'na gider; Program Files ve ProgramData içindeki klasörler yönetici izni ister.

### C: neden klasörlerinin toplamından fazla yer kullanıyor?

Windows, hiçbir klasör taramasının açamadığı ya da göremediği alan tutar: System Volume Information'daki geri yükleme noktaları ve gölge kopyalar, güncellemeler için ayrılmış depolama, NTFS dosya tablosu, disk belleği ve hazırda bekletme dosyaları. Alan haritasında sürücünün tamamını haritala; Gizli alan paneli her parçayı, haritanın onu sayıp saymadığını ve onu yöneten Windows ekranını (geri yükleme noktaları için Sistem Koruması, disk belleği için Performans Seçenekleri) gösterir. Windows geri yükleme noktalarının boyutunu yalnızca yöneticilere gösterdiği için ReClaim bunları ölçmek için bir kez izin ister ve yalnızca okur. Geriye kalan "Açıklanamayan" olarak işaretlenir: küçük dosyalar bütün bir disk kümesi kaplar, sabit bağlantılar birden fazla sayılır; bu yüzden harita ile sürücü hiçbir zaman tam tutmaz.

### WSL ya da Docker diskini nasıl küçültürüm?

`ext4.vhdx` ve `docker_data.vhdx` Linux veri yazdıkça büyür, içlerindeki dosyalar silinince küçülmez. ReClaim onları gösterir ama değiştirmez. Önce diski yedekle, sonra:

1. Linux içinde gereksizleri sil (Docker için `docker system prune`).
2. `wsl --shutdown` çalıştır.
3. Yönetici komut isteminde `diskpart` çalıştır, ardından `select vdisk file="<.vhdx yolu>"`, `attach vdisk readonly`, `compact vdisk`, `detach vdisk` ve `exit`. Hyper-V kuruluysa `Optimize-VHD -Path <yol> -Mode Full` aynı işi yapar.

Yeni WSL sürümleri diski seyrek tutup kendiliğinden küçültebilir: `wsl --manage <dağıtım> --set-sparse true`.

### ReClaim'i Dosya Gezgini'nin sağ tık menüsüne nasıl eklerim?

Ayarlar > Dosya Gezgini entegrasyonu > "Sağ tık menüsüne ReClaim ile analiz et ekle". Girdi klasörlerde, klasör arka planında ve sürücülerde yalnızca senin Windows kullanıcın için görünür ve yönetici izni gerektirmez. Windows 11'de "Diğer seçenekleri göster" (ya da Shift+F10) altındadır: yeni üst düzey menü yalnızca imzalı uygulama paketlerinden gelen girdileri kabul eder, ReClaim henüz böyle bir paket sunmuyor. Birkaç klasörü seçip girdiyi kullanırsan hepsi birlikte haritalanır.

Kaldırmak için anahtarı kapat. Kurulum (`.exe`) kaldırırken girdiyi siler, güncellemede bırakır. MSI kullanıcıya özel girdileri silmez (tüm kullanıcılar için kurar, girdi tek bir kullanıcıya aittir); MSI kurulumunu kaldırmadan önce anahtarı kapat. Kalan bir girdi yalnızca olmayan programı başlatamaz. Taşınabilir sürümde klasörü silmeden önce kapat. Anahtarlar: `HKCU\Software\Classes\Directory\shell\ReClaim.Analyze`, `...\Directory\Background\shell\ReClaim.Analyze` ve `...\Drive\shell\ReClaim.Analyze`.

### Bir dosya kullanımda olduğu için silinmiyor. Onu ne kullanıyor?

Büyük dosyalar'da, Alan haritasında ya da bir temizlikten sonra "Kullanımda" satırının yanında "Bunu ne kullanıyor?" seç. ReClaim, kurulum programlarının kullandığı Restart Manager ile dosyayı tutan uygulamaları, arka plan programlarını, hizmetleri ve Windows işlemlerini listeler. Bir klasör için ondan çalışan ya da onu çalışma klasörü olarak açık tutan (örneğin bir terminal) programları da bulur. "Uygulamayı kapat", uygulamadan kapatma düğmesine basılmış gibi kapanmasını ister; uygulama yine de kaydetmek isteyip istemediğini sorabilir. Hizmetler ve Windows işlemleri buradan durdurulmaz; ReClaim hiçbir zaman işlem sonlandırmaz ya da dosyaları zorla kapatmaz.

### Steam oyunlarım başka bir sürücüde. ReClaim onları buluyor mu?

Evet. Oyunlar sayfası ve Steam temizlik kuralları Steam'in kendi kütüphane listesini (`libraryfolders.vdf`) okur; Steam > Ayarlar > Depolama'dan eklediğin her kütüphane, hangi sürücüde olursa olsun, Steam klasörünün kendisiyle birlikte dahil edilir. Elle taşınıp junction ile geri bağlanan bir klasör izlenmez, çünkü ReClaim bağlantıları asla izlemez; onun yerine klasörü Steam'e kütüphane olarak ekle.

### Kayıt defterini temizliyor mu?

Hayır. Microsoft kayıt defteri temizleyicileri desteklemez, kazanılan alan da önemsizdir.

### Veri gönderiyor mu?

Hayır. Telemetri ve hesap yok. İsteğe bağlı güncelleme denetimi GitHub API'ye tek bir istek atar.

### Ayarlar ve geçmiş nerede duruyor?

`%APPDATA%\ReClaim`; kendi kuralların (`rules`) ve silinen kaldırma girdilerinin `.reg` yedekleri (`backups`) de orada. Taşınabilir sürümde exe'nin yanındaki `data` klasöründe.

Kurulum (`.exe`) sürümünü kaldırırken bu klasör yalnızca "Uygulama verilerini sil" kutusunu işaretlersen silinir. Yeni bir kurulum önce eski sürümü kaldırmayı önerirse aynı kutu çıkar; ayarlarını korumak için işaretleme. MSI bu klasörü hiç silmez.
