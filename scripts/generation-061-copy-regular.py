#!/usr/bin/env python3
"""Descriptor-based single-file evidence copy for trusted staging.

Replaces check-then-``cp`` staging, whose window between the destination
check and the copy allowed symlink/TOCTOU races. Guarantees:

- the source is opened with ``O_NOFOLLOW`` and must ``fstat`` as a regular
  file with one link — symlinks, FIFOs, directories, and hard-link tricks
  are refused at the descriptor, not by a racy path check;
- the destination is created with ``O_CREAT|O_EXCL|O_NOFOLLOW`` so an
  existing file or a symlink planted at the destination name always fails;
- the destination name must be a bare name (no separators) inside the
  stated directory, and the directory itself must not be a symlink;
- where the platform supports ``dir_fd`` (the hosted Linux runner), both
  opens are anchored ``openat``-style to directory descriptors so a parent
  directory swapped mid-flight cannot redirect the copy; on platforms
  without ``dir_fd`` support (macOS dev hosts) the same flag set applies to
  full paths, which still refuses final-component symlinks and existing
  destinations;
- source permission bits (including executable bits) are preserved on the
  copied file, never wider than 0o777;
- an optional ``--max-bytes`` bound refuses oversized evidence.

Portable to python3 >= 3.7.
"""

from __future__ import annotations

import argparse
import os
import stat
import sys

COPY_CHUNK_BYTES = 1024 * 1024

HAS_DIR_FD = os.open in os.supports_dir_fd


def fail(message: str) -> "int":
    print("generation-061-copy-regular: " + message, file=sys.stderr)
    return 1


def open_directory(path: str):
    flags = os.O_RDONLY | os.O_NOFOLLOW
    directory_flag = getattr(os, "O_DIRECTORY", 0)
    fd = os.open(path, flags | directory_flag)
    if not stat.S_ISDIR(os.fstat(fd).st_mode):
        os.close(fd)
        raise NotADirectoryError(path)
    return fd


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("source")
    parser.add_argument("destination_dir")
    parser.add_argument("name")
    parser.add_argument("--max-bytes", type=int, default=None)
    args = parser.parse_args()

    if os.sep in args.name or args.name in ("", ".", ".."):
        return fail("destination name must be a bare file name")

    source_dir, source_name = os.path.split(os.path.abspath(args.source))
    if not source_name:
        return fail("source must name a file")

    source_dir_fd = None
    destination_dir_fd = None
    source_fd = None
    destination_fd = None
    try:
        read_flags = os.O_RDONLY | os.O_NOFOLLOW
        create_flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW
        if HAS_DIR_FD:
            source_dir_fd = open_directory(source_dir)
            destination_dir_fd = open_directory(args.destination_dir)
            source_fd = os.open(source_name, read_flags, dir_fd=source_dir_fd)
            destination_fd = os.open(
                args.name, create_flags, 0o600, dir_fd=destination_dir_fd
            )
        else:
            if os.path.islink(args.destination_dir) or not os.path.isdir(
                args.destination_dir
            ):
                return fail("destination is not a real directory")
            source_fd = os.open(args.source, read_flags)
            destination_fd = os.open(
                os.path.join(args.destination_dir, args.name), create_flags, 0o600
            )

        source_stat = os.fstat(source_fd)
        if not stat.S_ISREG(source_stat.st_mode):
            return fail("source is not a regular file: " + args.source)
        if source_stat.st_nlink != 1:
            return fail("source has unexpected extra hard links: " + args.source)
        if args.max_bytes is not None and source_stat.st_size > args.max_bytes:
            return fail(
                "source exceeds the evidence size bound ({} > {} bytes): {}".format(
                    source_stat.st_size, args.max_bytes, args.source
                )
            )
        destination_stat = os.fstat(destination_fd)
        if not stat.S_ISREG(destination_stat.st_mode) or destination_stat.st_size != 0:
            return fail("destination descriptor is not a fresh regular file")

        copied = 0
        while True:
            chunk = os.read(source_fd, COPY_CHUNK_BYTES)
            if not chunk:
                break
            copied += len(chunk)
            if args.max_bytes is not None and copied > args.max_bytes:
                return fail("source grew past the evidence size bound mid-copy")
            written = 0
            while written < len(chunk):
                written += os.write(destination_fd, chunk[written:])
        os.fchmod(destination_fd, stat.S_IMODE(source_stat.st_mode) & 0o777)
    except FileExistsError:
        return fail("destination already exists: " + args.name)
    except (NotADirectoryError, IsADirectoryError):
        return fail("staging directories must be real directories")
    except OSError as error:
        return fail("descriptor staging refused the copy: " + str(error))
    finally:
        for fd in (source_fd, destination_fd, source_dir_fd, destination_dir_fd):
            if fd is not None:
                os.close(fd)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
