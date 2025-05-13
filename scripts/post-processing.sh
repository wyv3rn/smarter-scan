#!/usr/bin/env bash

process_page_image() {
    input_img=$1
    dpi=$2
    echo "Processing image ${input_img}, expecting a DPI of ${dpi}"

    actual_width=$(identify -format "%w" $input_img)
    expected_width=$(echo "w = 8.27 * ${dpi}; w / 1" | bc)
    if [ "$actual_width" -gt "$expected_width" ]; then
        width=$expected_width
        xoffset=$(echo "o = (${actual_width} - ${expected_width}) / 2; o / 1" | bc)
    else
        width=$actual_width
        xoffset=0
    fi
    height=$(echo "h = ${width} * 297 / 210; h / 1" | bc)

    cropped_img="${input_img}.cropped"
    convert "$input_img" -crop "${width}x${height}+${xoffset}+0" +repage $cropped_img
}


dpi=300
out="out.pdf"

while [[ $# -gt 0 ]]; do
    case $1 in
        -h|--help)
            echo "No help yet :("
            exit 0
            ;;
        -o|--output)
            out="$2"
            shift
            ;;
        --dpi)
            dpi="$2"
            shift
            ;;
        -*|--*)
            echo "Unknown option $1"
            exit -1
            ;;
        *)
            imgs+=("$1")
            imgs_cropped+=("${1}.cropped")
            [[ $1 == *front-* ]] && front_imgs_cropped+=("${1}.cropped")
            [[ $1 == *back-* ]] && back_imgs_cropped+=("${1}.cropped")
            ;;
    esac
    shift
done

n=${#imgs[@]}
m=${#front_imgs_cropped[@]}
k=${#back_imgs_cropped[@]}
((test_sum=$m+$k))

if [[ $k -gt 0 ]]; then
    echo "Enabling duplex mode because some pages are named *back-"
    if [[ $m -ne $k || $test_sum -ne $n ]]; then
        echo "Expected same number of front and back pages in duplex mode, and no other pages"
        exit -1
    fi
    # sort pages according to duplex logic
    imgs_cropped=()
    front_imgs_cropped=($(echo "${front_imgs_cropped[@]}" | tr " " "\n" | sort -V))
    back_imgs_cropped=($(echo "${back_imgs_cropped[@]}" | tr " " "\n" | sort -V -r))
    for ((i=0;i<$m;i++)); do
        imgs_cropped+=(${front_imgs_cropped[i]})
        imgs_cropped+=(${back_imgs_cropped[i]})
    done
fi

for img in ${imgs[@]}; do
    process_page_image $img $dpi
done

# combine all images to one PDF
echo "Combining all images to one PDF"
img2pdf ${imgs_cropped[@]} -o "${out}.combined"

# OCR
langs=$(tesseract --list-langs | grep -e "^eng\$" -e "^deu\$" | paste -d+ - -| sed 's/+$//')
echo "Doing OCR with the following languages: ${langs}"
ocrmypdf -l $langs --deskew "${out}.combined" $out

# cleanup
for img in ${imgs[@]}; do
    rm "${img}.cropped"
done
rm "${out}.combined"
