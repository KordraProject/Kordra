import { expect, test } from "@playwright/test";
import type { Download } from "../src/types";

test("unified workspace, filtering, task detail, adding, settings and themes", async ({page}) => {
  const stamp = "2026-11-27T10:00:00Z";
  const tasks: Download[] = [
    {id:"t1",source:"https://example.org/ubuntu-24.04.iso",engine:"aria2",status:"running",downloaded_bytes:1240000000,uploaded_bytes:0,total_bytes:5000000000,attempts:1,priority:1,error:null,created_at:stamp,updated_at:stamp},
    {id:"t2",source:"https://example.org/conference.mp4",engine:"yt-dlp",status:"queued",downloaded_bytes:0,uploaded_bytes:0,total_bytes:0,attempts:0,priority:0,error:null,created_at:stamp,updated_at:stamp},
    {id:"t3",source:"magnet:?xt=urn:btih:abcdef&dn=Linux%20distribution",engine:"librqbit",status:"completed",downloaded_bytes:1900000000,uploaded_bytes:980000000,total_bytes:1900000000,attempts:1,priority:0,error:null,created_at:stamp,updated_at:stamp},
    {id:"t4",source:"https://example.org/artwork-gallery",engine:"gallery-dl",status:"paused",downloaded_bytes:0,uploaded_bytes:0,total_bytes:0,attempts:0,priority:-1,error:null,created_at:stamp,updated_at:stamp},
    {id:"t5",source:"https://example.org/archive.zip",engine:"aria2",status:"failed",downloaded_bytes:2200000,uploaded_bytes:0,total_bytes:8000000,attempts:2,priority:0,error:"Connection timed out",created_at:stamp,updated_at:stamp},
  ];
  const errors: string[] = [];
  page.on("pageerror", e => errors.push(e.message));
  const settings = {concurrency:2,global_download_bps:null,yt_dlp_download_bps:null,torrent_download_bps:null,torrent_upload_bps:null};
  await page.route("**/api/v1/**", async route => {
    const request = route.request();
    const path = new URL(request.url()).pathname.replace("/api/v1","");
    let json: unknown;
    if(path === "/downloads" && request.method() === "POST") {
      const data = request.postDataJSON();
      const task = {...tasks[1],id:"new",source:data.source,engine:data.engine};
      tasks.unshift(task);json=task;
    } else if(path === "/downloads") json=tasks;
    else if(path === "/stats") json={total_downloaded_bytes:3140000000,total_uploaded_bytes:980000000,total_tasks:tasks.length,completed_tasks:1,failed_tasks:1,active_tasks:1};
    else if(path === "/stats/daily") json=Array.from({length:7},(_,i)=>({day:"2026-11-"+(20+i),downloaded_bytes:30000000*(i+1),uploaded_bytes:16000000*(i+1)}));
    else if(path === "/queue") json={queued:tasks.filter(t=>t.status==="queued").length,running:1,paused:1,max_concurrent:settings.concurrency,available_slots:1};
    else if(path === "/queue/settings") {if(request.method()==="PUT")Object.assign(settings,request.postDataJSON());json=settings;}
    else if(path === "/engines/yt-dlp") json={channel:"stable",active:"2026.11.01",fallback:null,pinned:null,auto_update:true,installed:["2026.11.01"]};
    else if(path === "/runtimes/deno") json={active:null,fallback:null,pinned:null,auto_update:true,installed:[]};
    else if(path === "/engines/lifecycle") json={policy:{startup:"cold",cold_retention:"persistent"},engines:[{id:"librqbit",kind:"embedded_service",ready:true},{id:"aria2",kind:"external_cli",ready:true}]};
    else if(path === "/downloads/t3/torrent") json={id:1,name:"Linux distribution",info_hash:"abcdef",paused:false,output_folder:"downloads",only_files:null,trackers:["https://example.org/tracker"],files:[{index:0,name:"linux.iso",length:1900000000,included:true,downloaded_bytes:1900000000}],stats:{progress_bytes:1900000000,uploaded_bytes:980000000,total_bytes:1900000000,finished:true,error:null,file_progress:[1900000000]}};
    else {await route.fulfill({status:404,json:{error:"not mocked"}});return;}
    await route.fulfill({json});
  });
  await page.goto("/");
  await expect(page.getByRole("heading",{name:"All downloads"})).toBeVisible();
  await expect(page.getByText("ubuntu-24.04.iso",{exact:true})).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute("data-theme","dark");
  await expect(page.locator("html")).toHaveCSS("--accent","#8b5cf6");
  await page.getByRole("button",{name:"Toggle row density"}).click();
  await expect(page.locator("html")).toHaveAttribute("data-density","compact");
  await page.getByRole("button",{name:"Toggle row density"}).click();
  await page.screenshot({path:"ui-screenshots/downloads-dark.png",fullPage:true});
  await page.getByRole("button",{name:"Toggle appearance"}).click();
  await expect(page.locator("html")).toHaveAttribute("data-theme","light");
  await page.screenshot({path:"ui-screenshots/downloads-light.png",fullPage:true});
  await page.getByRole("button",{name:"Completed",exact:true}).click();
  await expect(page.getByText("ubuntu-24.04.iso",{exact:true})).toHaveCount(0);
  await page.getByRole("button",{name:/Linux distribution/}).click();
  await expect(page.getByRole("complementary",{name:"Download details"})).toBeVisible();
  await expect(page.getByText("linux.iso",{exact:true})).toBeVisible();
  await page.screenshot({path:"ui-screenshots/torrent-inspector.png",fullPage:true});
  await page.getByRole("button",{name:"Close details",exact:true}).click();
  await page.getByRole("button",{name:"New download",exact:true}).click();
  await expect(page.getByRole("dialog")).toBeVisible();
  await page.getByPlaceholder("Paste a link…").fill("https://example.org/new-file.zip");
  await page.getByLabel("Download engine").selectOption("aria2");
  await page.getByRole("dialog").getByRole("button",{name:"Add download",exact:true}).click();
  await expect(page.getByText("new-file.zip",{exact:true})).toBeVisible();
  await page.getByLabel("Search downloads").fill("conference");
  await expect(page.getByText("new-file.zip",{exact:true})).toHaveCount(0);
  await page.getByRole("button",{name:"Clear search"}).click();
  await page.getByRole("button",{name:"Statistics",exact:true}).click();
  await expect(page.getByRole("heading",{name:"Transfer history"})).toBeVisible();
  await page.screenshot({path:"ui-screenshots/statistics.png",fullPage:true});
  await page.getByRole("button",{name:"Settings",exact:true}).click();
  await expect(page.getByRole("heading",{name:"Concurrency & speed limits"})).toBeVisible();
  await page.getByLabel("Parallel downloads (1–32)").fill("4");
  await page.getByRole("button",{name:"Apply live limits"}).click();
  await expect.poll(()=>settings.concurrency).toBe(4);
  await page.getByRole("button",{name:"All downloads",exact:true}).click();
  await page.setViewportSize({width:390,height:844});
  await page.screenshot({path:"ui-screenshots/mobile.png",fullPage:true});
  expect(errors).toEqual([]);
});
