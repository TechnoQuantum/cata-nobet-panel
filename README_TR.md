# Nöbetçi Eczane Paneli — CATA CT-4568

Windows uygulaması, 96 × 16 LED matrix paneli Bluetooth Low Energy üzerinden yönetir. Nöbet listesini elle girebilir, satır satır yapıştırabilir veya aylık Excel/ZIP dosyası içe aktarabilirsiniz. Seçili tarihin nöbetçi eczanelerini kayan yazı ve seçilen efektlerle panele gönderir.

## Başlatma

`NobetPanel.exe` dosyasını açın. İlk bağlantıda panel açık olmalı ve telefondaki iPixel Color uygulaması panelden ayrılmış olmalı. Panel bir BLE bağlantısını aynı anda kabul edebilir.

Uygulama kayıtları bu Windows hesabında `%LOCALAPPDATA%\NobetPanel\nobet_listesi.json` dosyasına kaydeder.

Uygulama içindeki **Windows açılışında küçültülmüş başlat** ayarı açıksa Windows oturum açılışında görev çubuğuna küçültülmüş olarak açılır. Aynı ayar günlük otomatik gönderimi de etkinleştirir. Uygulama paneli yaklaşık 5 saniyede bir tarar; tek tarama kaçırılması bağlantı kesintisi sayılmaz. Art arda iki taramada panel bulunamazsa çevrimdışı kabul edilir ve yeniden göründüğünde gönderim yapılır.

Nöbet tarihi vardiya saatine göre belirlenir: normal nöbet satırı kendi gününde 18:00’da başlar ve ertesi sabah 08:30’a kadar sürer. Pazar nöbeti Pazar 08:30’da başlar ve Pazartesi 08:30’a kadar devam eder. Böylece gece yarısı yeni günün kaydına erken geçilmez. 08:30–18:00 arasındaki boşlukta ekranda bugünün yaklaşan nöbeti, başlangıç ve bitiş saatiyle gösterilir; sabahı bitmiş önceki günün eczanesi ekranda kalmaz.

## Nöbet listesi

Tabloya tek kayıt girmek için tarih, eczane adı ve adresi doldurup **Kaydet / Güncelle** düğmesine basın. Aynı tarihteki mevcut kayıt güncellenir.

Toplu yapıştırma biçimi:

```text
2026-10-08 | Örnek Eczanesi | Merkez Eczanesi
09.10.2026 | Yeni Eczane
```

Ayraç olarak sekme, `|`, noktalı virgül veya virgül kullanabilirsiniz. Tarihten sonraki her sütun bir eczane adıdır. Birden fazla adı ` / ` ile elle de ayırabilirsiniz. Tarih biçimleri `YYYY-AA-GG`, `GG.AA.YYYY`, `GG/AA/YYYY` ve `GG-AA-YYYY` kabul edilir. CSV dışa aktarımı Excel uyumlu UTF-8 biçiminde yapılır.

### Aylık eczacı odası Excel listeleri

**Aylık Excel / ZIP içe al** düğmesi `.xlsx` veya içinde `.xlsx` dosyaları olan `.zip` arşivini açar. A sütunundaki gün, B ve C sütunlarındaki iki eczane adı aynı gün için birlikte kaydedilir. Başlık satırındaki yıl ve ay kullanılır; satırda parantez içinde başka ay/yıl açıkça yazıyorsa (ör. Aralık listesinin sonundaki 1 Ocak) o tarih de alınır. Kurum imzaları ve açılış/kapanış açıklamaları eczane kaydı olarak alınmaz.

Panele giden metin adres içermez. Örnek: `4 EKİM - BUGÜNÜN NÖBETÇİ ECZANELERİ: BİLGİ ECZANESİ - HAVACILAR ECZANESİ`.

## Panele gönderme

1. **Tara** düğmesiyle `LED_BLE_...` cihazını bulun. CT-4568’inizin adı `LED_BLE_4D9A9315`.
2. Tarih ve eczane kaydını ekleyin. Nöbet listesi bugüne ait olmalı.
3. İsterseniz yazı rengini, arka planı, efekti, hızı, parlaklığı ve TTF/OTF yazı tipini seçin.
4. **BUGÜNÜ PANELE GÖNDER** düğmesine basın.

Varsayılan metin adres içermez; eczane adlarını ve tarihi gösterir. Uzun metinler ekranda kayar. Uygulama açıkken gün değiştiğinde otomatik göndermeyi açabilirsiniz.

Normal nöbet listesinde kırmızı **E** eczane logosu isteğe bağlıdır; **Kayan nöbet listesinde E logosu göster (isteğe bağlı)** seçeneğiyle açıp kapatabilirsiniz. Logo sabit kalır ve kayan yazı kendi alanında, logonun yanında ilerler. E harfi çerçevenin üst çizgisine değmemesi için bir piksel aşağı konumlandırılmıştır.

Panel ayarlarında kendi eczane adınızı girip **Biz nöbetçiyken ‘BUGÜN NÖBETÇİYİZ’ göster** seçeneğini açabilirsiniz. Bugün eczane adı eşleşirse tarih ve `BUGÜN NÖBETÇİYİZ: [eczane adı]` mesajı sabit E logosunun yanında kayan yazı olarak gönderilir. Ad eşleşmezse o tarihin normal nöbet listesi gönderilir; bu listede logo yalnızca isteğe bağlı ayar açıksa görünür. Ön izleme yazıyı logo alanının dışına taşırmaz.

E logolu animasyon canlı ekrana tek sefer yüklenir ve GIF döngüsü sürekli oynatmayı ister. Seçili panele kayıt yuvasına animasyon yerine kısa kayan metin yedeği kaydedilir; bu, BLE üzerinden büyük GIF’in iki kez yüklenmesini önler. Kayan yazının başında vardiyanın tam tarih ve saat aralığı yer alır.

## Notlar

- Otomatik günlük gönderim için uygulamanın açık kalması ve bilgisayarın panelin Bluetooth menzilinde olması gerekir. Bilgisayar uykuya girerse tarama durur; Windows açıldığında uygulama tekrar başlar.
- BLE bağlantısı sırasında telefonun Bluetooth’unu veya iPixel Color uygulamasını kapatın.
- Küçük 96 × 16 panelde metnin tamamı aynı anda görünmez; logo sabit kalırken yazı zaman içinde kayar.
- Önizleme seçili tarih ve eczane adlarını, rengi ve efekti gösterir. Önizleme, gerçek panel görüntüsünü piksel piksel kopyalamaz; animasyon yönü ve hızı yaklaşık gösterilir.
- Bu uygulama CATA veya Heaton tarafından yayımlanmış resmî bir ürün değildir; CT-4568 ile eşleşen iPIXEL BLE arayüzünü kullanır.

## Kaynak ve protokol

Metin işleme ve BLE aktarımı açık kaynak `pypixelcolor` kütüphanesini kullanır (MIT lisansı). Cihaz UUID’leri `fa02` yazma ve `fa03` bildirim karakteristikleridir. [iPIXEL protokol belgeleri](https://github.com/cagcoach/ha-ipixel-color/blob/main/iPIXEL-Protocol-Documentation.md)
