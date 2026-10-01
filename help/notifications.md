# Notifications

Ferrum keeps a list of events under the bell, and can push the important ones to your phone.

## What is pushed

- A deploy refused or failed. The old version keeps running; the sentence says why.
- A deploy went live.
- Something broke on its own: a process systemd could not bring back, a certificate that could not be renewed, a Redis instance that failed to start.
- A Ferrum update is available.

Each kind has its own switch under Settings › About › Notifications. Everything else, such as a package dropped from the Aptfile or a route removed, stays in the list without a push.

## Turning it on

After you sign in, Ferrum offers to notify this device once. You can turn it on or off later under Settings, per device, and send yourself a test.

On an iPhone, notifications only work once Ferrum is on the Home Screen: open the panel in Safari, share, "Add to Home Screen", open it from there, then turn notifications on. Safari in a tab cannot receive them.

## How it travels

The message goes from your server to Apple's or Google's push relay to the phone, encrypted so only the phone can read it, and signed with a key made on your server at setup. No third-party service sees it. When a phone drops its subscription, Ferrum forgets that device on the next push.
