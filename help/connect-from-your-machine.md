# Connect from your machine

PostgreSQL on the box listens on loopback only, so a client on your machine reaches it through an SSH tunnel.

## The tunnel

The Databases page shows the command ready to copy:

```
ssh -L 5432:127.0.0.1:5432 ubuntu@your.server
```

The login is the user that installed Ferrum; change it under Settings › Connections if that is not the one you SSH in as. Root is refused on most boxes.

If Postgres already listens on 5432 on your machine, pick another local port and connect to that instead:

```
ssh -L 15432:127.0.0.1:5432 ubuntu@your.server
```

## The address

On the database's card, each role has a copy-URL control that gives you the full connection URL with its password. Paste it into your client and, when you tunnelled to another local port, change `5432` in it to that port. The panel needs your normal login for this; nothing else is asked.

## Redis

The same tunnel works for a Redis instance: its port is on the app's page, and the URL is in the app's environment under its label.
