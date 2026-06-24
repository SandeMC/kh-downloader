# Kingdom Hearts Downloader

Nasty and Complicated selective downloaders for Kingdom Hearts 1.5+2.5 from Steam and Epic Games Store, for Windows and Linux.
[add images later]

> [!CAUTION]
> I don't think this needs to be said, but this will not give you free access to the games. This will only download the games from your Steam or Epic Games account.
>
> That said, the app will require you to login for each download. No account details are sent anywhere beyond Steam or Epic and are not saved on your device.

## Usage

Just download the latest [release](https://github.com/SandeMC/kh-downloader/releases/latest) for your platform, launch it and follow the instructions. The buttons are self-explanatory... I think?

Report any issue at [Issues](https://github.com/SandeMC/kh-downloader/issues). The app may become out of date if the game updates - please inform me if it does.

## What is this app even for?

Square Enix does not allow you to choose what games to download. Say, you only want to play Kingdom Hearts II - you have to download the entire 65.42 GiB thing with every single game in it. Blasphemous - the game was only a little more than 4 GB on the PS2.

Well, you actually only need 17.65 GiB of downloading to play the Steam Version. That's still a lot more than 4GB, but that's also 47.7 GiB less than what Square asks you to download. Banger for anybody struggling with storage, slow internet speeds or limited traffic altogether!

(by the way you can also [downgrade the textures](https://www.nexusmods.com/kingdomhearts2finalmix/mods/249) and get the filesize down to around 11 GiB because Square AI upscaled the textures for no reason - still more than PS2, annoying)

## How

Basically, the games are actually not requiring eachother at all. They don't share any files beyond the Settings Menu. The launcher is literally unnecessary and only actually required by the movies - other games can be launched individually. If Square cared, they could've simply given you selective DLCs alike to Halo: Master Chief Collection to download the games selectively, but no they didn't care lol.

What I did is utilize steamroom-client (rust implementation of DepotDownloader), epic-api-rs and Epic-Asset-Manager (some code borrowed from there) to individually download the files from the official depots without ever downloading the files you don't need. You can do that with any game, but go learn DepotDownloader on your own for that.

## Game didn't appear on Steam

You installed the game via Steam but it didn't appear on Steam even after restarting it fully (Steam in top left - Exit)? Go to the location of your Steam install, go to the `steamapps` folder, create a `appmanifest_2552430.acf` file and paste these contents there:

```json
{
  "AppState": {
    "appid": "2552430",
    "Universe": "1",
    "name": "KINGDOM HEARTS -HD 1.5+2.5 ReMIX-",
    "StateFlags": "4",
    "installdir": "{install_dir_name}",
    "LastUpdated": "{timestamp_secs}",
    "UpdateResult": "0",
    "SizeOnDisk": "0",
    "buildid": "0",
    "AutoUpdateBehavior": "0",
    "InstalledDepots": {
      "2552433": {
        "manifest": "2946731077053901934",
        "size": "0"
      },
      "2552435": {
        "manifest": "3908821002986173448",
        "size": "0"
      }
    }
  }
}
```

Replace {install_dir_name} with your installation folder (make sure you replace \ and / with \\\) and {timestamp_secs} with an [accurate Unix timestamp](https://www.unixtimestamp.com) (or just put 1782260634 it doesn't really matter).
