# Разработка на другом ПК

Репозиторий содержит исходники Orkestr, Cargo.lock, тесты, документацию и макеты.
Старые программы, компиляторы, сборки, приватные базы, сертификаты и токены не входят
в проект и не нужны для сборки. Лицензии адаптированных MIT-исходников сохранены
в `licenses/` и описаны в `THIRD_PARTY_NOTICES.md`.

## Получить исходники

Установить [Git for Windows](https://git-scm.com/downloads/win) и
[Rust stable](https://www.rust-lang.org/tools/install). Для Windows MSVC нужны
Visual Studio Build Tools с C++ и Windows SDK. Рабочая копия:

```powershell
git clone https://github.com/a0gzy/Orkestr-EMP.git
Set-Location Orkestr-EMP
cargo build --workspace --locked
```

Cargo самостоятельно загрузит зависимости по Cargo.lock. Старые каталоги
`pdqdeployconsole`/`pdqinventoryconsole-2` и встроенный в них GCC не требуются.

## Проверки

С MSVC:

```powershell
.\tools\check-core.ps1
```

С GNU нужен отдельно установленный GCC/w64devkit:

```powershell
rustup toolchain install stable-x86_64-pc-windows-gnu
.\tools\check-core.ps1 -Gnu -CompilerBin 'C:\Tools\w64devkit\bin'
```

На исходном ПК сохранён один локальный w64devkit в
`%LOCALAPPDATA%\OrkestrEMP\toolchains\w64devkit`; `check-core.ps1 -Gnu` находит его
автоматически, если GCC отсутствует в PATH. Путь не зашит в Cargo-конфигурацию и не
должен копироваться вместе с проектом. Python 3 нужен только для дополнительных
инструментов `tools/validate.py`/создания синтетических UI fixtures, не для Rust build.

## Что переносить

Для продолжения разработки достаточно `git clone`; так не копируются гигабайты
`target`. Альтернатива — source ZIP из GitHub. Состояние последнего commit можно
экспортировать локально:

```powershell
git archive --format=zip --output=..\Orkestr-EMP-source.zip HEAD
```

`target/` — пересоздаваемый результат сборки. Его можно удалить командой `cargo clean`,
когда программы и сборка остановлены. После новой сборки он снова вырастет; это не
часть исходников. `.gitignore` исключает его, runtime data и секреты из Git.

Старые исходники/исследовательские материалы и приватные результаты пилота на исходном
ПК сохранены **отдельно от репозитория** в соседнем `Orkestr EMP local archive`.
Не добавлять этот архив в Git. В нём есть машинно привязанные DPAPI-данные: они не
работают на другом ПК/под другой identity и не являются переносимой dev-конфигурацией.

## Новый тестовый сервер

На новом ПК создать отдельный каталог данных; не использовать старые test credentials:

```powershell
.\target\debug\orkestr-server.exe init --data C:\OrkestrDevData --listen 127.0.0.1:7443 --dns localhost
.\target\debug\orkestr-server.exe run --data C:\OrkestrDevData
```

`init` требует новый каталог, создаёт собственные localhost certificate и DPAPI token.
Установки на ПК, настройка WinRM, firewall и SCM автоматически не выполняются.
Подключения и секреты вводятся заново. Подробности: [серверный контракт](m3-m5-server.md).

Имена ПК и домена в опубликованных отчётах заменены примерами `PC01`/`SERVER01`/
`example.test`; исходные локальные свидетельства остаются в приватном архиве.
